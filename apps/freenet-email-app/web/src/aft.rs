use std::sync::atomic::AtomicU32;
use std::{cell::RefCell, collections::HashMap};

use chrono::{DateTime, Utc};
use freenet_email_inbox::InboxParams;
use locutus_aft_interface::{
    AllocationCriteria, RequestNewToken, Tier, TokenAllocationRecord, TokenAllocationSummary,
    TokenAssignment, TokenDelegateMessage, TokenParameters,
};
use locutus_stdlib::client_api::{ContractRequest, DelegateRequest};
use locutus_stdlib::prelude::{
    ApplicationMessage, ContractInstanceId, ContractKey, DelegateKey, InboundDelegateMsg,
    Parameters, State, StateDelta, UpdateData,
};
use rsa::RsaPublicKey;

use crate::inbox::InboxModel;
use crate::{
    api::WebApiRequestClient,
    app::{error_handling, Identity, TryNodeAction},
    DynError,
};

pub(crate) static TOKEN_RECORD_CODE_HASH: &str =
    include_str!("../build/token_allocation_record_code_hash");
pub(crate) static TOKEN_GENERATOR_DELEGATE_CODE_HASH: &str =
    include_str!("../build/token_generator_code_hash");

pub(crate) struct AftRecords {}

type InboxContract = ContractKey;
type AftRecordId = ContractInstanceId;
type AftRecord = ContractKey;
type AftDelegate = DelegateKey;

type AssignmentHash = [u8; 32];

thread_local! {
    static RECORDS: RefCell<HashMap<Identity, TokenAllocationRecord>> = RefCell::new(HashMap::new());
    /// Contracts that require a token assingment still for a pending message.
    static PENDING_TOKEN_ASSIGNMENT: RefCell<HashMap<AftDelegate, Vec<InboxContract>>> = RefCell::new(HashMap::new());
    /// Assignments obtained from the delegate that need to be inserted in the record still.
    static PENDING_CONFIRMED_ASSIGNMENTS: RefCell<HashMap<AftRecordId, Vec<PendingAssignmentRegister>>> = RefCell::new(HashMap::new());
    /// A token which has been confirmed as valid by a contract update.
    static VALID_TOKEN: RefCell<HashMap<AftDelegate, Vec<TokenAssignment>>> = RefCell::new(HashMap::new());
    static PENDING_INBOXES_UPDATES: RefCell<Vec<(InboxContract, AssignmentHash)>> = RefCell::new(Vec::new());
}

// FIXME: we should check time to time in the API coroutine if any of the pending assignments
// have not been verified; if that's the case we may need to request again and cannot guarantee
// that a message has been delivered
struct PendingAssignmentRegister {
    /// time at which the request started
    start: DateTime<Utc>,
    // time_slot: DateTime<Utc>,
    // tier: locutus_aft_interface::Tier,
    record: TokenAssignment,
    requester: AftDelegate,
    inbox: InboxContract,
}

impl AftRecords {
    pub async fn load_all(
        client: &mut WebApiRequestClient,
        contracts: &[Identity],
        contract_to_id: &mut HashMap<AftRecord, Identity>,
    ) {
        for identity in contracts {
            let r = Self::load_contract(client, identity).await;
            if let Ok(key) = &r {
                contract_to_id.insert(key.clone(), identity.clone());
            }
            error_handling(
                client.clone().into(),
                r.map(|_| ()),
                TryNodeAction::LoadTokenRecord,
            )
            .await;
        }
    }

    async fn load_contract(
        client: &mut WebApiRequestClient,
        identity: &Identity,
    ) -> Result<AftRecord, DynError> {
        let params = TokenParameters::new(identity.key.to_public_key())
            .try_into()
            .map_err(|e| format!("{e}"))?;
        let contract_key =
            ContractKey::from_params(TOKEN_RECORD_CODE_HASH, params).map_err(|e| format!("{e}"))?;
        Self::get_state(client, contract_key.clone()).await?;
        Self::subscribe(client, contract_key.clone()).await?;
        Ok(contract_key)
    }

    pub fn pending_assignment(delegate: AftDelegate, contract: InboxContract) {
        PENDING_TOKEN_ASSIGNMENT.with(|map| {
            let map = &mut *map.borrow_mut();
            map.entry(delegate).or_default().push(contract);
        })
    }

    pub async fn confirm_allocation(
        client: &mut WebApiRequestClient,
        aft_record: AftRecordId,
        summary: TokenAllocationSummary,
    ) -> Result<(), DynError> {
        let Some(confirmed) = PENDING_CONFIRMED_ASSIGNMENTS.with(|pending| {
            let pending = &mut *pending.borrow_mut();
            pending.get_mut(&aft_record).and_then(|registers| {
                registers
                    .iter()
                    .position(|r| {
                        // fixme: the summary should also have the assignment hash
                        if summary.contains_alloc(r.record.tier, r.record.time_slot) {
                            return true;
                        }
                        false
                    })
                    .map(|idx| registers.remove(idx))
            })
        }) else { return Ok(()) };
        // we have a valid token now, so we can update the inbox contract
        InboxModel::finish_sending(client, confirmed.record, confirmed.inbox).await?;
        Ok(())
    }

    pub async fn allocated_assignment(
        client: &mut WebApiRequestClient,
        delegate_key: AftDelegate,
        record: TokenAssignment,
    ) -> Result<(), DynError> {
        let Some(inbox) = PENDING_INBOXES_UPDATES.with(|queue| {
            queue.borrow().iter().find_map(|(inbox, hash)| {
                if &record.assignment_hash == hash {
                    Some(inbox.clone())
                } else {
                    None
                }
            })
        }) else {
            // unexpected token
            return Ok(());
        };

        // update the token record contract for this delegate.
        let key = ContractKey::from(record.token_record);
        let request = ContractRequest::Update {
            key,
            data: UpdateData::Delta(serde_json::to_vec(&record)?.into()),
        };
        client.send(request.into()).await?;
        let token_record = record.token_record;

        let pending_register = PendingAssignmentRegister {
            start: Utc::now(),
            record,
            requester: delegate_key,
            inbox,
        };
        PENDING_CONFIRMED_ASSIGNMENTS.with(|pending| {
            let pending = &mut *pending.borrow_mut();
            pending
                .entry(token_record)
                .or_default()
                .push(pending_register);
        });
        Ok(())
    }

    pub async fn assign_token(
        client: &mut WebApiRequestClient,
        generator_id: &Identity,
        assignment_hash: [u8; 32],
    ) -> Result<DelegateKey, DynError> {
        static REQUEST_ID: AtomicU32 = AtomicU32::new(0);
        let sender_key = generator_id.key.to_public_key();
        let token_params: Parameters = TokenParameters::new(sender_key.clone())
            .try_into()
            .map_err(|e| format!("{e}"))
            .unwrap();
        let delegate_key = DelegateKey::from_params(
            crate::aft::TOKEN_GENERATOR_DELEGATE_CODE_HASH,
            token_params.clone(),
        )?;

        let inbox_params: Parameters = InboxParams {
            pub_key: sender_key.clone(),
        }
        .try_into()?;
        let inbox_key =
            ContractKey::from_params(crate::inbox::INBOX_CODE_HASH, inbox_params.clone())?;
        let delegate_params =
            locutus_aft_interface::DelegateParameters::new(generator_id.key.clone());

        let record_params = TokenParameters::new(sender_key.clone());
        let token_record: ContractInstanceId = ContractKey::from_params(
            crate::aft::TOKEN_RECORD_CODE_HASH,
            record_params.try_into()?,
        )
        .unwrap()
        .into();
        // todo: the criteria should come from the recipient inbox really
        let criteria = AllocationCriteria::new(
            Tier::Day1,
            std::time::Duration::from_secs(365 * 24 * 3600),
            token_record,
        )?;
        // fixme: should be using the state of the aft record contract here instead:
        let records = TokenAllocationRecord::new(HashMap::default());
        let token_request = TokenDelegateMessage::RequestNewToken(RequestNewToken {
            request_id: REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            delegate_id: delegate_key.clone().into(),
            criteria,
            records,
            assignee: sender_key.clone(),
            assignment_hash,
        });
        let request = DelegateRequest::ApplicationMessages {
            key: delegate_key.clone(),
            params: delegate_params.try_into()?,
            inbound: vec![InboundDelegateMsg::ApplicationMessage(
                ApplicationMessage::new(inbox_key.clone().into(), token_request.serialize()?),
            )],
        };
        PENDING_INBOXES_UPDATES.with(|queue| {
            queue.borrow_mut().push((inbox_key, assignment_hash));
        });
        client.send(request.into()).await?;

        // const TEST_TIER: Tier = Tier::Day1;
        // let naive = chrono::NaiveDate::from_ymd_opt(2023, 1, 25)
        //     .unwrap()
        //     .and_hms_opt(0, 0, 0)
        //     .unwrap();
        // let slot = DateTime::<Utc>::from_utc(naive, Utc);
        // crate::log::log(
        //     format!(
        //         "Sending update request message with token record key: {}",
        //         token_record.clone()
        //     )
        //     .as_str(),
        // );
        // Ok((
        //     delegate_key,
        //     TokenAssignment {
        //         tier: TEST_TIER,
        //         time_slot: slot,
        //         assignee: recipient_key,
        //         signature: rsa::pkcs1v15::Signature::from(vec![1u8; 64].into_boxed_slice()),
        //         assignment_hash: [0; 32],
        //         token_record,
        //     },
        // ))
        Ok(delegate_key)
    }

    // /// Add an inbox to the list of inboxes which is waiting for a valid token to complete an update.
    // pub fn needs_token(inbox: InboxContract) {}

    pub fn set(identity: Identity, state: State<'_>) -> Result<(), DynError> {
        let record = TokenAllocationRecord::try_from(state)?;
        RECORDS.with(|recs| {
            let recs = &mut *recs.borrow_mut();
            recs.insert(identity, record);
        });
        Ok(())
    }

    pub fn update_record(identity: Identity, delta: StateDelta<'_>) -> Result<(), DynError> {
        let record = TokenAllocationRecord::try_from(delta)?;
        RECORDS.with(|recs| {
            let recs = &mut *recs.borrow_mut();
            recs.insert(identity, record);
        });
        Ok(())
    }

    async fn get_state(client: &mut WebApiRequestClient, key: AftRecord) -> Result<(), DynError> {
        let request = ContractRequest::Get {
            key,
            fetch_contract: false,
        };
        client.send(request.into()).await?;
        Ok(())
    }

    async fn subscribe(client: &mut WebApiRequestClient, key: AftRecord) -> Result<(), DynError> {
        // todo: send the proper summary from the current state
        let request = ContractRequest::Subscribe { key, summary: None };
        client.send(request.into()).await?;
        Ok(())
    }
}