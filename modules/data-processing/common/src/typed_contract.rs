use locutus_stdlib::prelude::{ValidateResult, ContractInterface, Parameters, State, RelatedContracts, ContractError, StateDelta, StateSummary, UpdateData, UpdateModification};
use serde::{Serialize, Deserialize};
use bincode;

pub trait TypedContract {
    type State: Serialize + Deserialize<'static>;
    type StateDelta: Serialize + Deserialize<'static>;
    type StateSummary: Serialize + Deserialize<'static>;
    type UpdateData: Serialize + Deserialize<'static>;
    type UpdateModification: Serialize + Deserialize<'static>;
    type Parameters: Serialize + Deserialize<'static>;
    type RelatedContracts: Serialize + Deserialize<'static>;
    type ContractError: Serialize + Deserialize<'static>;

    fn validate_state(
        &self,
        parameters: Self::Parameters,
        state: Self::State,
        related: Self::RelatedContracts,
    ) -> Result<ValidateResult, Self::ContractError>;

    fn validate_delta(
        &self,
        parameters: Self::Parameters,
        delta: Self::StateDelta,
    ) -> Result<bool, Self::ContractError>;

    fn update_state(
        &self,
        parameters: Self::Parameters,
        state: Self::State,
        updates: Vec<Self::UpdateData>,
    ) -> Result<Self::UpdateModification, Self::ContractError>;

    fn summarize_state(
        &self,
        parameters: Self::Parameters,
        state: Self::State,
    ) -> Result<Self::StateSummary, Self::ContractError>;
}

