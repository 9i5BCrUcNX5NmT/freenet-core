use rust_fsm::{StateMachine, StateMachineImpl};

use crate::{conn_manager::ConnectionBridge, message::Transaction, node::OpStateStorage};

pub(crate) use self::messages::PutMsg;

use super::OpError;

/// This is just a placeholder for now!
pub(crate) struct PutOp(StateMachine<PutOpSM>);

impl PutOp {
    pub fn new(key: Vec<u8>, value: Vec<u8>) -> Self {
        let mut state = StateMachine::new();
        state.consume(&PutMsg::RouteValue { key, value }).unwrap();
        PutOp(state)
    }
}

struct PutOpSM;

impl StateMachineImpl for PutOpSM {
    type Input = PutMsg;

    type State = PutState;

    type Output = PutMsg;

    const INITIAL_STATE: Self::State = PutState::Initializing;

    fn transition(state: &Self::State, input: &Self::Input) -> Option<Self::State> {
        match (state, input) {
            (PutState::Initializing, PutMsg::RouteValue { key, .. }) => {
                Some(PutState::Requesting { key: key.clone() })
            }
            _ => None,
        }
    }

    fn output(state: &Self::State, input: &Self::Input) -> Option<Self::Output> {
        match (state, input) {
            (PutState::Initializing, PutMsg::RouteValue { key, value }) => {
                Some(PutMsg::RouteValue {
                    key: key.clone(),
                    value: value.clone(),
                })
            }
            _ => None,
        }
    }
}

enum PutState {
    Initializing,
    Requesting { key: Vec<u8> },
}

pub(crate) async fn put_op<CB>(
    op_storage: &mut OpStateStorage,
    conn_manager: &mut CB,
    put_op: PutMsg,
) -> Result<(), OpError>
where
    CB: ConnectionBridge,
{
    Ok(())
}

/// Request to insert/update a value into a contract.
pub(crate) async fn request_put<CB>(
    op_storage: &mut OpStateStorage,
    conn_manager: &mut CB,
    put_op: PutOp,
) -> Result<(), OpError>
where
    CB: ConnectionBridge,
{
    // the initial request must provide:
    // - a location in the network where the contract resides
    // - and the values to put
    todo!()
}

mod messages {
    use crate::conn_manager::PeerKeyLocation;

    use super::*;

    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
    pub(crate) enum PutMsg {
        RouteValue { key: Vec<u8>, value: Vec<u8> },
    }

    impl PutMsg {
        pub fn id(&self) -> &Transaction {
            todo!()
        }

        pub fn sender(&self) -> Option<&PeerKeyLocation> {
            todo!()
        }
    }
}