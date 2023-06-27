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