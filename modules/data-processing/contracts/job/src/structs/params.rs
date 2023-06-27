use locutus_stdlib::prelude::*;
use serde::*;

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



impl TryFrom<Parameters<'_>> for JobParams {
    type Error = ContractError;
    fn try_from(params: Parameters<'_>) -> Result<Self, Self::Error> {
        bincode::deserialize(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{}", err)))
    }
}

impl TryFrom<JobParams> for Parameters<'static> {
    type Error = bincode::ErrorKind;
    
    fn try_from(params: JobParams) -> Result<Self, Self::Error> {
        let serialized = bincode::serialize(&params)
            .map_err(|e| *e)?; // Convert from Box<ErrorKind> to ErrorKind

        Ok(Parameters::from(serialized)) // Convert Vec<u8> to Parameters
    }
}
