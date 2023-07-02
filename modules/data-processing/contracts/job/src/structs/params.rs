use locutus_stdlib::prelude::{*, bincode::*};
use serde::*;
use std::result::Result;

/// The parameters of this job, containing the JobSpec which contains the
/// WASM code to be executed.
#[derive(Serialize, Deserialize)]
pub struct JobParams {
    job_spec : JobSpec,
}

#[derive(Serialize, Deserialize)]
pub enum JobSpec {
    Wasm1 {
        wasm_job_parameters : Vec<u8>,
        wasm_job : Vec<u8>,
    }
}

/*
 * Convenience methods for converting between Parameters and JobParams
 */

impl TryFrom<Parameters<'_>> for JobParams {
    type Error = ContractError;
    fn try_from(params: Parameters<'_>) -> Result<Self, Self::Error> {
        deserialize(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{err}")))
    }
}

impl TryFrom<JobParams> for Parameters<'static> {
    type Error = Box<bincode::ErrorKind>;
    fn try_from(params: JobParams) -> Result<Self, Self::Error> {
        serialize(&params).map(Into::into)
    }
}
