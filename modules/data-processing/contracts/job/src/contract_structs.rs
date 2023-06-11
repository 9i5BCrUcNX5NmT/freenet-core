use antiflood_tokens::TokenAssignment;
use locutus_stdlib::prelude::*;
use p256::{
    ecdsa::{VerifyingKey, Signature},
};
use rsa::signature::Verifier;
use serde::{Serialize, Deserialize};

/// The state of the job contract contains the tokens authorizing
/// the job to be performed, and any results created so far
/// by a worker.
#[derive(Serialize, Deserialize)]
pub struct JobState {
    authorizing_tokens : Vec<TokenAssignment>,

    job_results : Vec<JobResult>,
}

/// The output of a job and related metadata.
#[derive(Serialize, Deserialize)]
pub struct JobResult {
    job_result : Vec<u8>,
    job_result_signature : Signature,
    worker : VerifyingKey,
}

impl JobResult {
    fn verify(&self) -> bool {
        self.worker.verify(self.job_result.as_slice(), &self.job_result_signature).is_ok()
    }
}

impl TryFrom<State<'_>> for JobState {
    type Error = ContractError;
    fn try_from(params: State<'_>) -> Result<Self, Self::Error> {
        serde_json::from_slice(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{err}")))
    }
}

impl TryFrom<JobState> for State<'static> {
    type Error = serde_json::Error;
    fn try_from(params: JobState) -> Result<Self, Self::Error> {
        serde_json::to_vec(&params).map(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct JobParams {
    job_spec : JobSpec,
}

impl TryFrom<Parameters<'_>> for JobParams {
    type Error = ContractError;
    fn try_from(params: Parameters<'_>) -> Result<Self, Self::Error> {
        serde_json::from_slice(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{err}")))
    }
}

impl TryFrom<JobParams> for Parameters<'static> {
    type Error = serde_json::Error;
    fn try_from(params: JobParams) -> Result<Self, Self::Error> {
        serde_json::to_vec(&params).map(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub enum JobSpec {
    Wasm1 {
        wasm_job_parameters : Vec<u8>,
        wasm_job : Vec<u8>,
    }
}

#[derive(Serialize, Deserialize)]
pub struct JobDelta {
}

impl TryFrom<StateDelta<'_>> for JobDelta {
    type Error = ContractError;
    fn try_from(params: StateDelta<'_>) -> Result<Self, Self::Error> {
        serde_json::from_slice(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{err}")))
    }
}

#[derive(Serialize, Deserialize)]
pub struct JobSummary {
}

impl TryFrom<StateSummary<'_>> for JobSummary {
    type Error = ContractError;
    fn try_from(params: StateSummary<'_>) -> Result<Self, Self::Error> {
        serde_json::from_slice(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{err}")))
    }
}