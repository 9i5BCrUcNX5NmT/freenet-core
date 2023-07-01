use std::{error::Error, fmt::{Formatter, self}, collections::HashMap};

use antiflood_tokens::TokenAssignment;
use locutus_stdlib::prelude::{*, blake2::Blake2s256};
use p256::{
    ecdsa::{VerifyingKey, Signature},
};
use blake3::Hash;
use serde::{Serialize, Deserialize, de::Visitor, Deserializer, Serializer};

/// The state of the job contract contains the tokens authorizing
/// the job to be performed, and any results created so far
/// by a worker.
#[derive(Serialize, Deserialize)]
pub struct JobState {
    /// To prevent spam a valid antiflood token must be provided and assigned to this
    /// job. The token may be no older than constants::MAXIMUM_JOB_ASSIGNMENT_TOKEN_AGE
    /// 
    /// This token may be replaced by any valid token that is newer than the current token
    /// and the same or a higher tier.
    /// 
    /// TODO: Should jobs be once-off or recurring like a cron job?
    pub authorizing_token : TokenAssignment,

    /// The output (or outputs if different workers disagree), up to a maximum of
    /// constants::MAX_JOB_OUTPUT_RECORDS.
    pub outputs : Vec<JobOutput>,
}

/// 
#[derive(Serialize, Deserialize)]
pub struct JobOutput {
    pub output : Vec<u8>,
    pub worker_verifications : Vec<WorkerVerification>,
}

/// The output of a job and related metadata.
#[derive(Serialize, Deserialize)]
pub struct WorkerVerification {
    job_output_signature : Signature,
    worker : VerifyingKey,
    antiflood_token : TokenAssignment,
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
