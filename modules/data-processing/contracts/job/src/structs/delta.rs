use common::utils::blake3_wrapper::Blake3HashWrapper;
use locutus_stdlib::prelude::*;
use serde::*;
use super::state::*;

#[derive(Serialize, Deserialize)]
pub struct JobDelta {
    job_outputs : Vec<JobOutput>,
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
    job_outputs : Vec<JobOutputSummary>,
}

#[derive(Serialize, Deserialize)]
pub struct JobOutputSummary {
    output_hash : Blake3HashWrapper,
    worker_verifications : Vec<WorkerVerification>,
}

impl TryFrom<StateSummary<'_>> for JobSummary {
    type Error = ContractError;
    fn try_from(params: StateSummary<'_>) -> Result<Self, Self::Error> {
        serde_json::from_slice(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{err}")))
    }
}