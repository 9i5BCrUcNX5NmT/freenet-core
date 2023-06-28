use std::collections::HashMap;

use super::structs::state::*;
use antiflood_tokens::TokenAllocationRecord;
use locutus_stdlib::prelude::*;

use crate::structs::params::JobParams;

impl ContractInterface for JobState {
    fn validate_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        related: RelatedContracts<'static>,
    ) -> Result<ValidateResult, ContractError> {
        let parameters : JobParams = parameters.try_into()?;
        let job_state : JobState = state.try_into()?;

        let mut missing_related = vec![];
        let mut allocation_records = HashMap::new();
        for (contract_id, state) in related.states() {
            let Some(state) = state else {
                missing_related.push(contract_id);
                continue;
            };
            let token_record = TokenAllocationRecord::try_from(state)?;
            allocation_records.insert(contract_id, token_record);
        }

        if !missing_related.is_empty() {
            return Ok(ValidateResult::RequestRelated(missing_related));
        }

        for token in job_state.authorizing_tokens {
            
        }

        todo!()
    }

    fn validate_delta(
        parameters: Parameters<'static>,
        delta: StateDelta<'static>,
    ) -> Result<bool, ContractError> {
        todo!()
    }

    fn update_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        data: Vec<UpdateData<'static>>,
    ) -> Result<UpdateModification<'static>, ContractError> {
        todo!()
    }

    fn summarize_state(
        parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        todo!()
    }

    fn get_state_delta(
        parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        todo!()
    }
}
