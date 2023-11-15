use std::{io, path::Path, time::SystemTime};

use chrono::{DateTime, Utc};
use either::Either;
use freenet_stdlib::prelude::*;
use futures::{future::BoxFuture, FutureExt};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::OpenOptions,
    io::AsyncSeekExt,
    sync::{
        mpsc::{self},
        Mutex,
    },
};

use super::PeerKey;
use crate::{
    config::GlobalExecutor,
    contract::StoreResponse,
    message::{Message, Transaction},
    operations::{connect, get::GetMsg, put::PutMsg, subscribe::SubscribeMsg},
    ring::{Location, PeerKeyLocation},
    router::RouteEvent,
    DynError,
};

#[cfg(feature = "trace-ot")]
pub(super) use opentelemetry_tracer::OTEventRegister;
#[cfg(test)]
pub(super) use test::TestEventListener;

use super::OpManager;

#[derive(Debug, Clone, Copy)]
struct ListenerLogId(usize);

/// A type that reacts to incoming messages from the network and records information about them.
pub(crate) trait NetEventRegister: std::any::Any + Send + Sync + 'static {
    fn register_events<'a>(
        &'a mut self,
        events: Either<NetEventLog<'a>, Vec<NetEventLog<'a>>>,
    ) -> BoxFuture<'a, ()>;
    fn trait_clone(&self) -> Box<dyn NetEventRegister>;
    fn as_any(&self) -> &dyn std::any::Any
    where
        Self: Sized,
    {
        self as _
    }
    fn notify_of_time_out(&mut self, tx: Transaction) -> BoxFuture<()>;
}

#[cfg(feature = "trace-ot")]
pub(crate) struct CombinedRegister<const N: usize>([Box<dyn NetEventRegister>; N]);

#[cfg(feature = "trace-ot")]
impl<const N: usize> CombinedRegister<N> {
    pub fn new(registries: [Box<dyn NetEventRegister>; N]) -> Self {
        Self(registries)
    }
}

#[cfg(feature = "trace-ot")]
impl<const N: usize> NetEventRegister for CombinedRegister<N> {
    fn register_events<'a>(
        &'a mut self,
        events: Either<NetEventLog<'a>, Vec<NetEventLog<'a>>>,
    ) -> BoxFuture<'a, ()> {
        async move {
            for registry in &mut self.0 {
                registry.register_events(events.clone()).await;
            }
        }
        .boxed()
    }

    fn trait_clone(&self) -> Box<dyn NetEventRegister> {
        Box::new(self.clone())
    }

    fn notify_of_time_out(&mut self, tx: Transaction) -> BoxFuture<()> {
        async move {
            for reg in &mut self.0 {
                reg.notify_of_time_out(tx);
            }
        }
        .boxed()
    }
}

#[cfg(feature = "trace-ot")]
impl<const N: usize> Clone for CombinedRegister<N> {
    fn clone(&self) -> Self {
        let mut i = 0;
        let cloned: [Box<dyn NetEventRegister>; N] = [None::<()>; N].map(|_| {
            let cloned = self.0[i].trait_clone();
            i += 1;
            cloned
        });
        Self(cloned)
    }
}

#[derive(Clone)]
pub(crate) struct NetEventLog<'a> {
    tx: &'a Transaction,
    peer_id: &'a PeerKey,
    kind: EventKind,
}

impl<'a> NetEventLog<'a> {
    pub fn route_event(
        tx: &'a Transaction,
        op_storage: &'a OpManager,
        route_event: &RouteEvent,
    ) -> Self {
        NetEventLog {
            tx,
            peer_id: &op_storage.ring.peer_key,
            kind: EventKind::Route(route_event.clone()),
        }
    }

    pub fn disconnected(from: &'a PeerKey) -> Self {
        NetEventLog {
            tx: Transaction::NULL,
            peer_id: from,
            kind: EventKind::Disconnected,
        }
    }

    pub fn from_outbound_msg(
        msg: &'a Message,
        op_storage: &'a OpManager,
    ) -> Either<Self, Vec<Self>> {
        let kind = match msg {
            Message::Connect(connect::ConnectMsg::Response {
                msg:
                    connect::ConnectResponse::AcceptedBy {
                        peers,
                        your_location,
                        your_peer_id,
                    },
                ..
            }) => {
                let this_peer = op_storage.ring.own_location();
                if peers.contains(&this_peer) {
                    EventKind::Connect(ConnectEvent::Connected {
                        this: this_peer,
                        connected: PeerKeyLocation {
                            peer: *your_peer_id,
                            location: Some(*your_location),
                        },
                    })
                } else {
                    EventKind::Ignored
                }
            }
            Message::Connect(connect::ConnectMsg::Response {
                msg:
                    connect::ConnectResponse::Proxy {
                        accepted_by,
                        joiner,
                    },
                ..
            }) => {
                let this_peer = op_storage.ring.own_location();
                if accepted_by.contains(&this_peer) {
                    EventKind::Connect(ConnectEvent::Connected {
                        this: this_peer,
                        connected: PeerKeyLocation {
                            peer: *joiner,
                            location: None,
                        },
                    })
                } else {
                    EventKind::Ignored
                }
            }
            _ => EventKind::Ignored,
        };
        Either::Left(NetEventLog {
            tx: msg.id(),
            peer_id: &op_storage.ring.peer_key,
            kind,
        })
    }

    pub fn from_inbound_msg(
        msg: &'a Message,
        op_storage: &'a OpManager,
    ) -> Either<Self, Vec<Self>> {
        let kind = match msg {
            Message::Connect(connect::ConnectMsg::Response {
                msg:
                    connect::ConnectResponse::AcceptedBy {
                        peers,
                        your_location,
                        your_peer_id,
                    },
                ..
            }) => {
                let this_peer = &op_storage.ring.peer_key;
                let mut events = peers
                    .iter()
                    .map(|peer| {
                        let kind: EventKind = EventKind::Connect(ConnectEvent::Connected {
                            this: PeerKeyLocation {
                                peer: *your_peer_id,
                                location: Some(*your_location),
                            },
                            connected: *peer,
                        });
                        NetEventLog {
                            tx: msg.id(),
                            peer_id: this_peer,
                            kind,
                        }
                    })
                    .collect::<Vec<_>>();
                if this_peer == your_peer_id {
                    events.push(NetEventLog {
                        tx: msg.id(),
                        peer_id: this_peer,
                        kind: EventKind::Connect(ConnectEvent::Finished {
                            initiator: *your_peer_id,
                            location: *your_location,
                        }),
                    });
                }
                return Either::Right(events);
            }
            Message::Put(PutMsg::RequestPut {
                contract, target, ..
            }) => {
                let key = contract.key();
                EventKind::Put(PutEvent::Request {
                    performer: target.peer,
                    key,
                })
            }
            Message::Put(PutMsg::SuccessfulUpdate { new_value, .. }) => {
                EventKind::Put(PutEvent::PutSuccess {
                    requester: op_storage.ring.peer_key,
                    value: new_value.clone(),
                })
            }
            Message::Put(PutMsg::Broadcasting {
                new_value,
                broadcast_to,
                key,
                ..
            }) => EventKind::Put(PutEvent::BroadcastEmitted {
                broadcast_to: broadcast_to.clone(),
                key: key.clone(),
                value: new_value.clone(),
            }),
            Message::Put(PutMsg::BroadcastTo {
                sender,
                new_value,
                key,
                ..
            }) => EventKind::Put(PutEvent::BroadcastReceived {
                requester: sender.peer,
                key: key.clone(),
                value: new_value.clone(),
            }),
            Message::Get(GetMsg::ReturnGet {
                key,
                value: StoreResponse { state: Some(_), .. },
                ..
            }) => EventKind::Get { key: key.clone() },
            Message::Subscribe(SubscribeMsg::ReturnSub {
                subscribed: true,
                key,
                sender,
                ..
            }) => EventKind::Subscribed {
                key: key.clone(),
                at: *sender,
            },
            _ => EventKind::Ignored,
        };
        Either::Left(NetEventLog {
            tx: msg.id(),
            peer_id: &op_storage.ring.peer_key,
            kind,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
struct NetLogMessage {
    tx: Transaction,
    datetime: DateTime<Utc>,
    peer_id: PeerKey,
    kind: EventKind,
}

impl NetLogMessage {
    fn to_log_message<'a>(
        log: Either<NetEventLog<'a>, Vec<NetEventLog<'a>>>,
    ) -> impl Iterator<Item = NetLogMessage> + Send + 'a {
        let erased_iter = match log {
            Either::Left(one) => Box::new([one].into_iter())
                as Box<dyn std::iter::Iterator<Item = NetEventLog<'_>> + Send + 'a>,
            Either::Right(multiple) => Box::new(multiple.into_iter())
                as Box<dyn std::iter::Iterator<Item = NetEventLog<'_>> + Send + 'a>,
        };
        erased_iter.into_iter().map(NetLogMessage::from)
    }

    /// Signals whether this message closes a transaction span.
    ///
    /// In case of isolated events where the span is not being tracked it should return true.
    #[cfg(feature = "trace-ot")]
    fn span_completed(&self) -> bool {
        match &self.kind {
            EventKind::Connect(ConnectEvent::Finished { .. }) => true,
            EventKind::Connect(_) => false,
            EventKind::Put(PutEvent::PutSuccess { .. }) => true,
            EventKind::Put(_) => false,
            _ => false,
        }
    }
}

impl<'a> From<NetEventLog<'a>> for NetLogMessage {
    fn from(log: NetEventLog<'a>) -> NetLogMessage {
        NetLogMessage {
            datetime: Utc::now(),
            tx: *log.tx,
            kind: log.kind,
            peer_id: *log.peer_id,
        }
    }
}

#[cfg(feature = "trace-ot")]
impl<'a> From<&'a NetLogMessage> for Option<Vec<opentelemetry::KeyValue>> {
    fn from(msg: &'a NetLogMessage) -> Self {
        use opentelemetry::KeyValue;
        let map: Option<Vec<KeyValue>> = match &msg.kind {
            EventKind::Connect(ConnectEvent::StartConnection { from }) => Some(vec![
                KeyValue::new("phase", "start"),
                KeyValue::new("initiator", format!("{from}")),
            ]),
            EventKind::Connect(ConnectEvent::Connected { this, connected }) => Some(vec![
                KeyValue::new("phase", "connected"),
                KeyValue::new("from", format!("{this}")),
                KeyValue::new("to", format!("{connected}")),
            ]),
            EventKind::Connect(ConnectEvent::Finished {
                initiator,
                location,
            }) => Some(vec![
                KeyValue::new("phase", "finished"),
                KeyValue::new("initiator", format!("{initiator}")),
                KeyValue::new("location", location.as_f64()),
            ]),
            _ => None,
        };
        map.map(|mut map| {
            map.push(KeyValue::new("peer_id", format!("{}", msg.peer_id)));
            map
        })
    }
}

#[derive(Clone)]
pub(crate) struct EventRegister {
    log_sender: mpsc::Sender<NetLogMessage>,
}

/// Records from a new session must have higher than this ts.
static NEW_RECORDS_TS: std::sync::OnceLock<SystemTime> = std::sync::OnceLock::new();
static FILE_LOCK: Mutex<()> = Mutex::const_new(());

impl EventRegister {
    #[cfg(not(test))]
    const MAX_LOG_RECORDS: usize = 100_000;
    #[cfg(test)]
    const MAX_LOG_RECORDS: usize = 10_000;

    pub fn new() -> Self {
        let (log_sender, log_recv) = mpsc::channel(1000);
        NEW_RECORDS_TS.get_or_init(SystemTime::now);
        GlobalExecutor::spawn(Self::record_logs(log_recv));
        Self { log_sender }
    }

    async fn record_logs(mut log_recv: mpsc::Receiver<NetLogMessage>) {
        const BATCH_SIZE: usize = 100;

        async fn num_lines(path: &Path) -> io::Result<usize> {
            use tokio::fs::File;
            use tokio::io::AsyncReadExt;

            let mut file = tokio::io::BufReader::new(File::open(path).await?);
            let mut num_records = 0;
            let mut buf = [0; 4]; // Read the u32 length prefix

            loop {
                let bytes_read = file.read_exact(&mut buf).await;
                if bytes_read.is_err() {
                    break;
                }
                num_records += 1;

                // Seek to the next record without reading its contents
                let length = u32::from_le_bytes(buf) as u64;
                if (file.seek(io::SeekFrom::Current(length as i64)).await).is_err() {
                    break;
                }
            }

            Ok(num_records)
        }

        async fn truncate_records(
            file: &mut tokio::fs::File,
            remove_records: usize,
        ) -> Result<(), Box<dyn std::error::Error>> {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let _guard = FILE_LOCK.lock().await;
            file.rewind().await?;
            // tracing::debug!(position = file.stream_position().await.unwrap());
            let mut records_count = 0;
            while records_count < remove_records {
                let mut length_bytes = [0u8; 4];
                if let Err(error) = file.read_exact(&mut length_bytes).await {
                    if matches!(error.kind(), io::ErrorKind::UnexpectedEof) {
                        break;
                    }
                    let pos = file.stream_position().await;
                    tracing::error!(%error, ?pos, "error while trying to read file");
                    return Err(error.into());
                }
                let length = u32::from_be_bytes(length_bytes);
                if let Err(error) = file.seek(io::SeekFrom::Current(length as i64)).await {
                    if matches!(error.kind(), io::ErrorKind::UnexpectedEof) {
                        break;
                    }
                    let pos = file.stream_position().await;
                    tracing::error!(%error, ?pos, "error while trying to read file");
                    return Err(error.into());
                }
                records_count += 1;
            }

            // Copy the rest of the file to the buffer
            let mut buffer = Vec::new();
            if let Err(error) = file.read_to_end(&mut buffer).await {
                if !matches!(error.kind(), io::ErrorKind::UnexpectedEof) {
                    let pos = file.stream_position().await;
                    tracing::error!(%error, ?pos, "error while trying to read file");
                    return Err(error.into());
                }
            }

            #[cfg(test)]
            {
                assert!(!buffer.is_empty());
                let mut unique = std::collections::HashSet::new();
                let mut read_buf = &*buffer;
                let mut length_bytes: [u8; 4] = [0u8; 4];
                let mut cursor = 0;
                while read_buf.read_exact(&mut length_bytes).await.is_ok() {
                    let length = u32::from_be_bytes(length_bytes) as usize;
                    cursor += 4;
                    let log: NetLogMessage =
                        bincode::deserialize(&buffer[cursor..cursor + length]).unwrap();
                    cursor += length;
                    read_buf = &buffer[cursor..];
                    unique.insert(log.peer_id);
                    // tracing::debug!(?log, %cursor);
                }
                assert!(unique.len() > 1);
            }

            // Seek back to the beginning and write the remaining content
            file.rewind().await?;
            file.write_all(&buffer).await?;

            // Truncate the file to the new size
            file.set_len(buffer.len() as u64).await?;
            file.seek(io::SeekFrom::End(0)).await?;
            Ok(())
        }

        let event_log_path = crate::config::Config::conf().event_log();
        tracing::info!(?event_log_path);
        let mut event_log = match OpenOptions::new()
            .write(true)
            .read(true)
            .open(&event_log_path)
            .await
        {
            Ok(file) => file,
            Err(err) => {
                tracing::error!("Failed openning log file {:?} with: {err}", event_log_path);
                panic!("Failed openning log file"); // fixme: propagate this to the main thread
            }
        };
        let mut num_written = 0;
        let mut batch_buf = vec![];
        let mut log_batch = Vec::with_capacity(BATCH_SIZE);

        let mut num_recs = num_lines(event_log_path.as_path())
            .await
            .expect("non IO error");

        while let Some(log) = log_recv.recv().await {
            log_batch.push(log);

            if log_batch.len() >= BATCH_SIZE {
                let num_logs: usize = log_batch.len();
                let moved_batch = std::mem::replace(&mut log_batch, Vec::with_capacity(BATCH_SIZE));
                let serialization_task = tokio::task::spawn_blocking(move || {
                    let mut batch_serialized_data = Vec::with_capacity(BATCH_SIZE * 1024);
                    for log_item in &moved_batch {
                        let mut serialized = match bincode::serialize(log_item) {
                            Err(err) => {
                                tracing::error!("Failed serializing log: {err}");
                                return Err(err);
                            }
                            Ok(serialized) => serialized,
                        };
                        {
                            use byteorder::{BigEndian, WriteBytesExt};
                            batch_serialized_data
                                .write_u32::<BigEndian>(serialized.len() as u32)
                                .expect("enough memory");
                        }
                        batch_serialized_data.append(&mut serialized);
                    }
                    Ok(batch_serialized_data)
                });

                match serialization_task.await {
                    Ok(Ok(serialized_data)) => {
                        // tracing::debug!(bytes = %serialized_data.len(), %num_logs, "serialized logs");
                        batch_buf = serialized_data;
                        num_written += num_logs;
                        log_batch.clear(); // Clear the batch for new data
                    }
                    _ => {
                        panic!("Failed serializing log");
                    }
                }
            }

            if num_written >= BATCH_SIZE {
                {
                    use tokio::io::AsyncWriteExt;
                    let _guard = FILE_LOCK.lock().await;
                    if let Err(err) = event_log.write_all(&batch_buf).await {
                        tracing::error!("Failed writting to event log: {err}");
                        panic!("Failed writting event log");
                    }
                }
                num_recs += num_written;
                num_written = 0;
            }

            // Check the number of lines and truncate if needed
            if num_recs > Self::MAX_LOG_RECORDS {
                const REMOVE_RECS: usize = 1000 + BATCH_SIZE; // making space for 1000 new records
                if let Err(err) = truncate_records(&mut event_log, REMOVE_RECS).await {
                    tracing::error!("Failed truncating log file: {:?}", err);
                    panic!("Failed truncating log file");
                }
                num_recs -= REMOVE_RECS;
            }
        }
    }

    pub async fn get_router_events(max_event_number: usize) -> Result<Vec<RouteEvent>, DynError> {
        use tokio::io::AsyncReadExt;
        const MAX_EVENT_HISTORY: usize = 10_000;
        let event_num = max_event_number.min(MAX_EVENT_HISTORY);

        let event_log_path = crate::config::Config::conf().event_log();
        // tracing::info!(?event_log_path);
        let _guard: tokio::sync::MutexGuard<'_, ()> = FILE_LOCK.lock().await;
        let mut file =
            tokio::io::BufReader::new(OpenOptions::new().read(true).open(event_log_path).await?);

        let new_records_ts = NEW_RECORDS_TS
            .get()
            .expect("set on initialization")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("should be older than unix epoch")
            .as_secs() as i64;

        let mut records = Vec::with_capacity(event_num);
        while records.len() < event_num {
            // Read the length prefix
            let length = match file.read_u32().await {
                Ok(l) => l,
                Err(error) => {
                    if !matches!(error.kind(), io::ErrorKind::UnexpectedEof) {
                        let pos = file.stream_position().await;
                        tracing::error!(%error, ?pos, "error while trying to read file");
                        return Err(error.into());
                    } else {
                        break;
                    }
                }
            };
            let mut buf = vec![0; length as usize];
            file.read_exact(&mut buf).await?;
            records.push(buf);
            if records.len() == event_num {
                break;
            }
        }

        let deserialized_records = tokio::task::spawn_blocking(move || {
            let mut filtered = vec![];
            for buf in records {
                let record: NetLogMessage = bincode::deserialize(&buf).map_err(|e| {
                    tracing::error!(?buf, "deserialization error");
                    e
                })?;
                // tracing::info!(?record);
                if let EventKind::Route(outcome) = record.kind {
                    let record_ts = record.datetime.timestamp();
                    if record_ts >= new_records_ts {
                        filtered.push(outcome);
                    }
                }
            }
            Ok::<_, DynError>(filtered)
        })
        .await??;

        Ok(deserialized_records)
    }
}

impl NetEventRegister for EventRegister {
    fn register_events<'a>(
        &'a mut self,
        logs: Either<NetEventLog<'a>, Vec<NetEventLog<'a>>>,
    ) -> BoxFuture<'a, ()> {
        async {
            for log_msg in NetLogMessage::to_log_message(logs) {
                let _ = self.log_sender.send(log_msg).await;
            }
        }
        .boxed()
    }

    fn trait_clone(&self) -> Box<dyn NetEventRegister> {
        Box::new(self.clone())
    }

    fn notify_of_time_out(&mut self, _: Transaction) -> BoxFuture<()> {
        async {}.boxed()
    }
}

#[cfg(feature = "trace-ot")]
mod opentelemetry_tracer {
    #[cfg(not(test))]
    use std::collections::HashMap;
    use std::time::Duration;

    use dashmap::DashMap;
    use opentelemetry::{
        global,
        trace::{self, Span},
    };

    use super::*;

    struct OTSpan {
        inner: global::BoxedSpan,
        last_log: SystemTime,
    }

    impl OTSpan {
        fn new(transaction: Transaction) -> Self {
            use trace::{Tracer, TracerProvider};

            let tracer = {
                let tracer_provider = global::tracer_provider();
                tracer_provider.versioned_tracer(
                    "freenet",
                    Some(env!("CARGO_PKG_VERSION")),
                    Some("https://opentelemetry.io/schemas/1.21.0"),
                    None,
                )
            };
            let tx_bytes = transaction.as_bytes();
            let mut span_id = [0; 8];
            span_id.copy_from_slice(&tx_bytes[8..]);
            let start_time = transaction.started();
            let inner = tracer.build(trace::SpanBuilder {
                name: transaction.transaction_type().description().into(),
                start_time: Some(start_time),
                span_id: Some(trace::SpanId::from_bytes(span_id)),
                trace_id: Some(trace::TraceId::from_bytes(tx_bytes)),
                ..Default::default()
            });
            OTSpan {
                inner,
                last_log: SystemTime::now(),
            }
        }

        fn add_log(&mut self, log: &NetLogMessage) {
            // NOTE: if we need to add some standard attributes in the future take a look at
            // https://docs.rs/opentelemetry-semantic-conventions/latest/opentelemetry_semantic_conventions/
            let ts = SystemTime::UNIX_EPOCH
                + Duration::from_nanos(
                    ((log.datetime.timestamp() * 1_000_000_000)
                        + log.datetime.timestamp_subsec_nanos() as i64) as u64,
                );
            self.last_log = ts;
            if let Some(log_vals) = <Option<Vec<_>>>::from(log) {
                self.inner.add_event_with_timestamp(
                    log.tx.transaction_type().description(),
                    ts,
                    log_vals,
                );
            }
        }
    }

    impl Drop for OTSpan {
        fn drop(&mut self) {
            self.inner.end_with_timestamp(self.last_log);
        }
    }

    impl trace::Span for OTSpan {
        delegate::delegate! {
            to self.inner {
                fn span_context(&self) -> &trace::SpanContext;
                fn is_recording(&self) -> bool;
                fn set_attribute(&mut self, attribute: opentelemetry::KeyValue);
                fn set_status(&mut self, status: trace::Status);
                fn end_with_timestamp(&mut self, timestamp: SystemTime);
            }
        }

        fn add_event_with_timestamp<T>(
            &mut self,
            _: T,
            _: SystemTime,
            _: Vec<opentelemetry::KeyValue>,
        ) where
            T: Into<std::borrow::Cow<'static, str>>,
        {
            unreachable!("not explicitly called")
        }

        fn update_name<T>(&mut self, _: T)
        where
            T: Into<std::borrow::Cow<'static, str>>,
        {
            unreachable!("shouldn't change span name")
        }
    }

    #[derive(Clone)]
    pub(in crate::node) struct OTEventRegister {
        log_sender: mpsc::Sender<NetLogMessage>,
        finished_tx_notifier: mpsc::Sender<Transaction>,
    }

    /// For tests running in a single process is importart that span tracking is global across threads and simulated peers.  
    static UNIQUE_REGISTER: std::sync::OnceLock<DashMap<Transaction, OTSpan>> =
        std::sync::OnceLock::new();

    impl OTEventRegister {
        pub fn new() -> Self {
            if cfg!(test) {
                UNIQUE_REGISTER.get_or_init(DashMap::new);
            }
            let (sender, finished_tx_notifier) = mpsc::channel(100);
            let (log_sender, log_recv) = mpsc::channel(1000);
            NEW_RECORDS_TS.get_or_init(SystemTime::now);
            GlobalExecutor::spawn(Self::record_logs(log_recv, finished_tx_notifier));
            Self {
                log_sender,
                finished_tx_notifier: sender,
            }
        }

        async fn record_logs(
            mut log_recv: mpsc::Receiver<NetLogMessage>,
            mut finished_tx_notifier: mpsc::Receiver<Transaction>,
        ) {
            #[cfg(not(test))]
            let mut logs = HashMap::new();

            #[cfg(not(test))]
            fn process_log(logs: &mut HashMap<Transaction, OTSpan>, log: NetLogMessage) {
                let span_completed = log.span_completed();
                match logs.entry(log.tx) {
                    std::collections::hash_map::Entry::Occupied(mut val) => {
                        {
                            let span = val.get_mut();
                            span.add_log(&log);
                        }
                        if span_completed {
                            let (_, _span) = val.remove_entry();
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(empty) => {
                        let span = empty.insert(OTSpan::new(log.tx));
                        // does not make much sense to treat a single isolated event as a span,
                        // so just ignore those in case they were to happen
                        if !span_completed {
                            span.add_log(&log);
                        }
                    }
                }
            }

            #[cfg(test)]
            fn process_log(logs: &DashMap<Transaction, OTSpan>, log: NetLogMessage) {
                let span_completed = log.span_completed();
                match logs.entry(log.tx) {
                    dashmap::mapref::entry::Entry::Occupied(mut val) => {
                        {
                            let span = val.get_mut();
                            span.add_log(&log);
                        }
                        if span_completed {
                            let (_, _span) = val.remove_entry();
                        }
                    }
                    dashmap::mapref::entry::Entry::Vacant(empty) => {
                        let mut span = empty.insert(OTSpan::new(log.tx));
                        // does not make much sense to treat a single isolated event as a span,
                        // so just ignore those in case they were to happen
                        if !span_completed {
                            span.add_log(&log);
                        }
                    }
                }
            }

            #[cfg(not(test))]
            fn cleanup_timed_out(logs: &mut HashMap<Transaction, OTSpan>, tx: Transaction) {
                if let Some(_span) = logs.remove(&tx) {}
            }

            #[cfg(test)]
            fn cleanup_timed_out(logs: &DashMap<Transaction, OTSpan>, tx: Transaction) {
                if let Some((_, _span)) = logs.remove(&tx) {}
            }

            loop {
                tokio::select! {
                    log_msg = log_recv.recv() => {
                        if let Some(log) = log_msg {
                            #[cfg(not(test))]
                            {
                                process_log(&mut logs, log);
                            }
                            #[cfg(test)]
                            {
                                process_log(UNIQUE_REGISTER.get().expect("should be set"), log);
                            }
                        } else {
                            break;
                        }
                    }
                    finished_tx = finished_tx_notifier.recv() => {
                        if let Some(tx) = finished_tx {
                            #[cfg(not(test))]
                            {
                                cleanup_timed_out(&mut logs, tx);
                            }
                            #[cfg(test)]
                            {
                                cleanup_timed_out(UNIQUE_REGISTER.get().expect("should be set"), tx);
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    }

    impl NetEventRegister for OTEventRegister {
        fn register_events<'a>(
            &'a mut self,
            logs: Either<NetEventLog<'a>, Vec<NetEventLog<'a>>>,
        ) -> BoxFuture<'a, ()> {
            async {
                for log_msg in NetLogMessage::to_log_message(logs) {
                    let _ = self.log_sender.send(log_msg).await;
                }
            }
            .boxed()
        }

        fn trait_clone(&self) -> Box<dyn NetEventRegister> {
            Box::new(self.clone())
        }

        fn notify_of_time_out(&mut self, tx: Transaction) -> BoxFuture<()> {
            async move {
                if cfg!(test) {
                    let _ = self.finished_tx_notifier.send(tx).await;
                }
            }
            .boxed()
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
// todo: make this take by ref instead, probably will need an owned version
enum EventKind {
    Connect(ConnectEvent),
    Put(PutEvent),
    // todo: make this a sequence like Put
    Get {
        key: ContractKey,
    },
    Route(RouteEvent),
    // todo: add update sequences too
    Subscribed {
        key: ContractKey,
        at: PeerKeyLocation,
    },
    Ignored,
    Disconnected,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
enum ConnectEvent {
    StartConnection {
        from: PeerKey,
    },
    Connected {
        this: PeerKeyLocation,
        connected: PeerKeyLocation,
    },
    Finished {
        initiator: PeerKey,
        location: Location,
    },
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
enum PutEvent {
    Request {
        performer: PeerKey,
        key: ContractKey,
    },
    PutSuccess {
        requester: PeerKey,
        value: WrappedState,
    },
    BroadcastEmitted {
        /// subscribed peers
        broadcast_to: Vec<PeerKeyLocation>,
        /// key of the contract which value was being updated
        key: ContractKey,
        /// value that was put
        value: WrappedState,
    },
    BroadcastReceived {
        /// peer who started the broadcast op
        requester: PeerKey,
        /// key of the contract which value was being updated
        key: ContractKey,
        /// value that was put
        value: WrappedState,
    },
}

#[cfg(test)]
pub(super) mod test {
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicUsize, Ordering::SeqCst},
            Arc,
        },
        time::Duration,
    };

    use dashmap::DashMap;
    use parking_lot::Mutex;

    use super::*;
    use crate::{node::tests::NodeLabel, ring::Distance};

    static LOG_ID: AtomicUsize = AtomicUsize::new(0);

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn event_register_read_write() -> Result<(), DynError> {
        let event_log_path = crate::config::Config::conf().event_log();
        // truncate the log if it exists
        std::fs::File::create(event_log_path).unwrap();

        // force a truncation
        const TEST_LOGS: usize = EventRegister::MAX_LOG_RECORDS + 100;
        let mut register = EventRegister::new();
        let bytes = crate::util::test::random_bytes_2mb();
        let mut gen = arbitrary::Unstructured::new(&bytes);
        let mut transactions = vec![];
        let mut peers = vec![];
        let mut events = vec![];
        for _ in 0..TEST_LOGS {
            let tx: Transaction = gen.arbitrary()?;
            transactions.push(tx);
            let peer: PeerKey = gen.arbitrary()?;
            peers.push(peer);
        }
        for _ in 0..TEST_LOGS {
            let kind: EventKind = gen.arbitrary()?;
            events.push(NetEventLog {
                tx: transactions.last().unwrap(),
                peer_id: peers.last().unwrap(),
                kind,
            });
        }
        register.register_events(Either::Right(events)).await;
        while register.log_sender.capacity() != 1000 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        tokio::time::sleep(Duration::from_millis(3_000)).await;
        let ev = EventRegister::get_router_events(100).await?;
        assert!(!ev.is_empty());
        Ok(())
    }

    #[derive(Clone)]
    pub(crate) struct TestEventListener {
        node_labels: Arc<DashMap<NodeLabel, PeerKey>>,
        tx_log: Arc<DashMap<Transaction, Vec<ListenerLogId>>>,
        logs: Arc<Mutex<Vec<NetLogMessage>>>,
    }

    impl TestEventListener {
        pub fn new() -> Self {
            TestEventListener {
                node_labels: Arc::new(DashMap::new()),
                tx_log: Arc::new(DashMap::new()),
                logs: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn add_node(&mut self, label: NodeLabel, peer: PeerKey) {
            self.node_labels.insert(label, peer);
        }

        pub fn is_connected(&self, peer: &PeerKey) -> bool {
            let logs = self.logs.lock();
            logs.iter().any(|log| {
                &log.peer_id == peer
                    && matches!(log.kind, EventKind::Connect(ConnectEvent::Connected { .. }))
            })
        }

        pub fn has_put_contract(
            &self,
            peer: &PeerKey,
            for_key: &ContractKey,
            expected_value: &WrappedState,
        ) -> bool {
            let logs = self.logs.lock();
            let put_ops = logs.iter().filter_map(|l| match &l.kind {
                EventKind::Put(ev) => Some((&l.tx, ev)),
                _ => None,
            });
            let put_ops: HashMap<_, Vec<_>> = put_ops.fold(HashMap::new(), |mut acc, (id, ev)| {
                acc.entry(id).or_default().push(ev);
                acc
            });

            for (_tx, events) in put_ops {
                let mut is_expected_value = false;
                let mut is_expected_key = false;
                let mut is_expected_peer = false;
                for ev in events {
                    match ev {
                        PutEvent::Request { key, .. } if key != for_key => break,
                        PutEvent::Request { key, .. } if key == for_key => {
                            is_expected_key = true;
                        }
                        PutEvent::PutSuccess { requester, value }
                            if requester == peer && value == expected_value =>
                        {
                            is_expected_peer = true;
                            is_expected_value = true;
                        }
                        _ => {}
                    }
                }
                if is_expected_value && is_expected_peer && is_expected_key {
                    return true;
                }
            }
            false
        }

        /// The contract was broadcasted from one peer to an other successfully.
        pub fn contract_broadcasted(&self, for_key: &ContractKey) -> bool {
            let logs = self.logs.lock();
            let put_broadcast_ops = logs.iter().filter_map(|l| match &l.kind {
                EventKind::Put(ev @ PutEvent::BroadcastEmitted { .. })
                | EventKind::Put(ev @ PutEvent::BroadcastReceived { .. }) => Some((&l.tx, ev)),
                _ => None,
            });
            let put_broadcast_by_tx: HashMap<_, Vec<_>> =
                put_broadcast_ops.fold(HashMap::new(), |mut acc, (id, ev)| {
                    acc.entry(id).or_default().push(ev);
                    acc
                });
            for (_tx, events) in put_broadcast_by_tx {
                let mut was_emitted = false;
                let mut was_received = false;
                for ev in events {
                    match ev {
                        PutEvent::BroadcastEmitted { key, .. } if key.clone() == *for_key => {
                            was_emitted = true;
                        }
                        PutEvent::BroadcastReceived { key, .. } if key.clone() == *for_key => {
                            was_received = true;
                        }
                        _ => {}
                    }
                }
                if was_emitted && was_received {
                    return true;
                }
            }
            false
        }

        pub fn has_got_contract(&self, peer: &PeerKey, expected_key: &ContractKey) -> bool {
            let logs = self.logs.lock();
            logs.iter().any(|log| {
                &log.peer_id == peer
                    && matches!(log.kind, EventKind::Get { ref key } if key == expected_key  )
            })
        }

        pub fn is_subscribed_to_contract(
            &self,
            peer: &PeerKey,
            expected_key: &ContractKey,
        ) -> bool {
            let logs = self.logs.lock();
            logs.iter().any(|log| {
                &log.peer_id == peer
                    && matches!(log.kind, EventKind::Subscribed { ref key, .. } if key == expected_key  )
            })
        }

        /// Unique connections for a given peer and their relative distance to other peers.
        pub fn connections(&self, peer: PeerKey) -> impl Iterator<Item = (PeerKey, Distance)> {
            let logs = self.logs.lock();
            let disconnects = logs
                .iter()
                .filter(|l| matches!(l.kind, EventKind::Disconnected))
                .fold(HashMap::<_, Vec<_>>::new(), |mut map, log| {
                    map.entry(log.peer_id).or_default().push(log.datetime);
                    map
                });

            logs.iter()
                .filter_map(|l| {
                    if let EventKind::Connect(ConnectEvent::Connected { this, connected }) = l.kind
                    {
                        let disconnected = disconnects
                            .get(&connected.peer)
                            .iter()
                            .flat_map(|dcs| dcs.iter())
                            .any(|dc| dc > &l.datetime);
                        if let Some((this_loc, conn_loc)) = this.location.zip(connected.location) {
                            if this.peer == peer && !disconnected {
                                return Some((connected.peer, conn_loc.distance(this_loc)));
                            }
                        }
                    }
                    None
                })
                .collect::<HashMap<_, _>>()
                .into_iter()
        }

        fn create_log(log: NetEventLog) -> (NetLogMessage, ListenerLogId) {
            let log_id = ListenerLogId(LOG_ID.fetch_add(1, SeqCst));
            let NetEventLog { peer_id, kind, .. } = log;
            let msg_log = NetLogMessage {
                datetime: Utc::now(),
                tx: *log.tx,
                peer_id: *peer_id,
                kind,
            };
            (msg_log, log_id)
        }
    }

    impl super::NetEventRegister for TestEventListener {
        fn register_events<'a>(
            &'a mut self,
            logs: Either<NetEventLog<'a>, Vec<NetEventLog<'a>>>,
        ) -> BoxFuture<'a, ()> {
            match logs {
                Either::Left(log) => {
                    let tx = log.tx;
                    let (msg_log, log_id) = Self::create_log(log);
                    self.logs.lock().push(msg_log);
                    self.tx_log.entry(*tx).or_default().push(log_id);
                }
                Either::Right(logs) => {
                    let logs_list = &mut *self.logs.lock();
                    for log in logs {
                        let tx = log.tx;
                        let (msg_log, log_id) = Self::create_log(log);
                        logs_list.push(msg_log);
                        self.tx_log.entry(*tx).or_default().push(log_id);
                    }
                }
            }
            async {}.boxed()
        }

        fn trait_clone(&self) -> Box<dyn NetEventRegister> {
            Box::new(self.clone())
        }

        fn notify_of_time_out(&mut self, _: Transaction) -> BoxFuture<()> {
            async {}.boxed()
        }
    }

    #[test]
    fn test_get_connections() -> Result<(), anyhow::Error> {
        use crate::ring::Location;
        let peer_id = PeerKey::random();
        let loc = Location::try_from(0.5)?;
        let tx = Transaction::new::<connect::ConnectMsg>();
        let locations = [
            (PeerKey::random(), Location::try_from(0.5)?),
            (PeerKey::random(), Location::try_from(0.75)?),
            (PeerKey::random(), Location::try_from(0.25)?),
        ];

        let mut listener = TestEventListener::new();
        locations.iter().for_each(|(other, location)| {
            listener.register_events(Either::Left(NetEventLog {
                tx: &tx,
                peer_id: &peer_id,
                kind: EventKind::Connect(ConnectEvent::Connected {
                    this: PeerKeyLocation {
                        peer: peer_id,
                        location: Some(loc),
                    },
                    connected: PeerKeyLocation {
                        peer: *other,
                        location: Some(*location),
                    },
                }),
            }));
        });

        let distances: Vec<_> = listener.connections(peer_id).collect();
        assert!(distances.len() == 3);
        assert!(
            (distances.iter().map(|(_, l)| l.as_f64()).sum::<f64>() - 0.5f64).abs() < f64::EPSILON
        );
        Ok(())
    }
}