use antiflood_tokens::TokenAssignment;
use locutus_stdlib::prelude::*;
use p256::{
    ecdsa::{VerifyingKey, Signature},
};

/// The state of the job contract contains the tokens authorizing
/// the job to be performed, and any results created so far
/// by a worker.
pub struct JobState {
    authorizing_tokens : Vec<TokenAssignment>,

    job_results : Vec<JobResult>,
}

/// The output of a job and related metadata.
pub struct JobResult {
    job_result : Vec<u8>,
    job_result_signature : Signature,
    worker : VerifyingKey,
}

/// 
pub struct JobParams {
    job_spec : JobSpec,
}

pub enum JobSpec {
    Wasm1 {
        wasm_job_parameters : Vec<u8>,
        wasm_job : Vec<u8>,
    }
}

impl ContractctInterface for JobState {
    fn validate_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        related: RelatedContracts<'static>,
    ) -> Result<ValidateResult, ContractError> {
        todo!()
    }

    fn validate_delta(
        _parameters: Parameters<'static>,
        delta: StateDelta<'static>,
    ) -> Result<bool, ContractError> {
        todo!();
    }

    fn update_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        updates: Vec<UpdateData<'static>>,
    ) -> Result<UpdateModification<'static>, ContractError> {
        todo!()
    }

    fn summarize_state(
        _parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        todo!()
    }

    fn get_state_delta(
        _parameters: Parameters<'static>,
        state: State<'static>,
        _summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        todo!()
    }
}
