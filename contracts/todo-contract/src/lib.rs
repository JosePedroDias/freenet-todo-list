use freenet_stdlib::prelude::*;
use todo_common::TodoState;

struct Contract;

#[contract]
impl ContractInterface for Contract {
    fn validate_state(
        _parameters: Parameters<'static>,
        state: State<'static>,
        _related: RelatedContracts<'static>,
    ) -> Result<ValidateResult, ContractError> {
        if state.as_ref().is_empty() {
            return Ok(ValidateResult::Valid);
        }
        let todo: TodoState = serde_json::from_slice(state.as_ref())
            .map_err(|e| ContractError::Deser(e.to_string()))?;
        if todo.validate() {
            Ok(ValidateResult::Valid)
        } else {
            Err(ContractError::InvalidState)
        }
    }

    fn update_state(
        _parameters: Parameters<'static>,
        state: State<'static>,
        data: Vec<UpdateData<'static>>,
    ) -> Result<UpdateModification<'static>, ContractError> {
        let mut todo: TodoState = if state.is_empty() {
            TodoState::default()
        } else {
            serde_json::from_slice(state.as_ref())
                .map_err(|e| ContractError::Deser(e.to_string()))?
        };

        for ud in data {
            match ud {
                UpdateData::Delta(d) => {
                    if d.is_empty() {
                        continue;
                    }
                    let incoming: TodoState = serde_json::from_slice(d.as_ref())
                        .map_err(|e| ContractError::Deser(e.to_string()))?;
                    todo.merge(incoming);
                }
                UpdateData::State(s) => {
                    if s.is_empty() {
                        continue;
                    }
                    let incoming: TodoState = serde_json::from_slice(s.as_ref())
                        .map_err(|e| ContractError::Deser(e.to_string()))?;
                    todo.merge(incoming);
                }
                UpdateData::StateAndDelta { state, delta } => {
                    if !state.is_empty() {
                        let incoming: TodoState = serde_json::from_slice(state.as_ref())
                            .map_err(|e| ContractError::Deser(e.to_string()))?;
                        todo.merge(incoming);
                    }
                    if !delta.is_empty() {
                        let incoming: TodoState = serde_json::from_slice(delta.as_ref())
                            .map_err(|e| ContractError::Deser(e.to_string()))?;
                        todo.merge(incoming);
                    }
                }
                _ => return Err(ContractError::InvalidUpdate),
            }
        }

        let out = serde_json::to_vec(&todo).map_err(|e| ContractError::Other(e.to_string()))?;
        Ok(UpdateModification::valid(State::from(out)))
    }

    fn summarize_state(
        _parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        Ok(StateSummary::from(state.as_ref().to_vec()))
    }

    fn get_state_delta(
        _parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        let mut todo: TodoState = if state.is_empty() {
            TodoState::default()
        } else {
            serde_json::from_slice(state.as_ref())
                .map_err(|e| ContractError::Deser(e.to_string()))?
        };
        if !summary.is_empty() {
            let summ: TodoState = serde_json::from_slice(summary.as_ref())
                .map_err(|e| ContractError::Deser(e.to_string()))?;
            todo.merge(summ);
        }
        let out = serde_json::to_vec(&todo).map_err(|e| ContractError::Other(e.to_string()))?;
        Ok(StateDelta::from(out))
    }
}
