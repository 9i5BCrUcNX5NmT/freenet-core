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

    /// The highest priority job_results are processed first.
    pub job_results : (Vec<u8>, Vec<JobResult>),
}

/// The output of a job and related metadata.
#[derive(Serialize, Deserialize)]
pub struct JobResult {
    job_result_signature : Signature,
    worker : VerifyingKey,
    antiflood_token : TokenAssignment,
}

enum JobOutput {
    Data(Vec<u8>),
    Hash(Hash),
}

impl JobResult {
    pub fn verify(&self) -> bool {
       // self.worker.verify(self.job_result.as_slice(), &self.job_result_signature).is_ok()
        todo!()
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

struct JobOutputVisitor;

impl<'de> Visitor<'de> for JobOutputVisitor {
    type Value = JobOutput;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str("a byte vector or a hash string")
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(JobOutput::Data(value.to_vec()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let hash = Hash::from_hex(value).map_err(E::custom)?;
        Ok(JobOutput::Hash(hash))
    }
}

impl Serialize for JobOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            JobOutput::Data(ref vec) => vec.serialize(serializer),
            JobOutput::Hash(ref hash) => serializer.serialize_bytes(hash.as_bytes()),
        }
    }
}

impl<'de> Deserialize<'de> for JobOutput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(JobOutputVisitor)
    }
}