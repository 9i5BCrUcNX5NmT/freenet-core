use antiflood_tokens::TokenAssignment;
use locutus_stdlib::prelude::{*, bincode::*};
use p256::{
    ecdsa::{VerifyingKey, Signature},
};
use serde::{Serialize, Deserialize};
use std::result::Result;

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

/// One specific output of a job, along with a list of the workers who agree with this
/// output.
#[derive(Serialize, Deserialize)]
pub struct JobOutput {
    /// The output bytes of the job.
    pub output : Vec<u8>,

    // The workers who agree with this output.
    pub worker_verifications : Vec<WorkerVerification>,
}

/// A record that a worker has verified this output of a job.
#[derive(Serialize, Deserialize)]
pub struct WorkerVerification {
    /// The time this verification was generated, a worker can submit multiple
    /// verifications for the same output, all but the most recent will be
    /// ignored. This is to allow workers to update their verification if,
    /// for example, an input contract state changes.
    verification_time : chrono::DateTime<chrono::Utc>,
    
    /// The signature of the worker who verified this output
    job_output_signature : Signature,
    
    /// The public key of the worker who verified this output
    worker : VerifyingKey,

    /// An antiflood token that was valid at the time of verification
    antiflood_token : TokenAssignment,
}

/*
 * Convenience methods for converting between State and JobState
 */

impl TryFrom<State<'_>> for JobState {
    type Error = ContractError;
    fn try_from(params: State<'_>) -> Result<Self, Self::Error> {
        deserialize(params.as_ref())
            .map_err(|err| ContractError::Deser(format!("{err}")))
    }
}

impl TryFrom<JobState> for State<'static> {
    type Error = Box<bincode::ErrorKind>;
    fn try_from(params: JobState) -> Result<Self, Self::Error> {
        serialize(&params).map(Into::into)
    }
}
