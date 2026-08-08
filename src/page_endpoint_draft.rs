use std::collections::BTreeSet;

use crate::{
    EndpointInputLocation, PageEndpointDefinition, endpoint_identifier_is_valid,
    validate_route_path,
};

pub(crate) fn validate_page_endpoint_draft(
    endpoint: &PageEndpointDefinition,
    siblings: &[PageEndpointDefinition],
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = validate_route_path(&endpoint.path) {
        errors.push(error.to_string());
    }
    if siblings.iter().any(|sibling| {
        sibling.id != endpoint.id
            && sibling.method == endpoint.method
            && sibling.path == endpoint.path
    }) {
        errors.push(format!(
            "接口路由重复: {} {}",
            endpoint.method.as_str(),
            endpoint.path
        ));
    }

    let mut input_names = BTreeSet::new();
    for input in &endpoint.inputs {
        if !endpoint_identifier_is_valid(&input.name) || !input_names.insert(input.name.as_str()) {
            errors.push(format!("入参标识无效或重复: {}", input.name));
        }
        if input.location == EndpointInputLocation::Path
            && !endpoint.path.contains(&format!("{{{}}}", input.name))
        {
            errors.push(format!("REST 路径缺少参数 {{{}}}", input.name));
        }
    }

    let mut output_names = BTreeSet::new();
    for output in &endpoint.outputs {
        if !endpoint_identifier_is_valid(&output.name) || !output_names.insert(output.name.as_str())
        {
            errors.push(format!("响应字段标识无效或重复: {}", output.name));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use crate::{
        DefinitionState, EndpointInputDefinition, EndpointOutputDefinition, RestMethod, SymbolId,
        ValueType,
    };

    use super::*;

    fn endpoint(path: &str) -> PageEndpointDefinition {
        PageEndpointDefinition {
            id: SymbolId::new(),
            title: String::new(),
            description: String::new(),
            state: DefinitionState::Known,
            implementation: crate::EndpointImplementationDefinition::Convention,
            method: RestMethod::Post,
            path: path.to_owned(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    #[test]
    fn validates_route_identity_and_structured_parameters() {
        let existing = endpoint("/api/users/batch-disable");
        let mut draft = endpoint("/api/users/batch-disable");
        draft.inputs = vec![
            EndpointInputDefinition {
                id: SymbolId::new(),
                name: "user_id".to_owned(),
                title: "用户".to_owned(),
                location: EndpointInputLocation::Path,
                value_type: ValueType::Text,
                required: true,
            },
            EndpointInputDefinition {
                id: SymbolId::new(),
                name: "user_id".to_owned(),
                title: "重复用户".to_owned(),
                location: EndpointInputLocation::Body,
                value_type: ValueType::Text,
                required: false,
            },
        ];
        draft.outputs = vec![EndpointOutputDefinition {
            id: SymbolId::new(),
            name: "Invalid-Name".to_owned(),
            title: "无效响应".to_owned(),
            value_type: ValueType::Text,
        }];

        let errors = validate_page_endpoint_draft(&draft, &[existing]);

        assert!(errors.iter().any(|error| error.contains("路由重复")));
        assert!(errors.iter().any(|error| error.contains("路径缺少参数")));
        assert!(errors.iter().any(|error| error.contains("入参标识")));
        assert!(errors.iter().any(|error| error.contains("响应字段标识")));
    }

    #[test]
    fn accepts_matching_path_parameter_and_current_route_identity() {
        let mut draft = endpoint("/api/users/{userId}");
        draft.method = RestMethod::Get;
        draft.inputs.push(EndpointInputDefinition {
            id: SymbolId::new(),
            name: "userId".to_owned(),
            title: "用户".to_owned(),
            location: EndpointInputLocation::Path,
            value_type: ValueType::Text,
            required: true,
        });
        draft.outputs.push(EndpointOutputDefinition {
            id: SymbolId::new(),
            name: "displayName".to_owned(),
            title: "姓名".to_owned(),
            value_type: ValueType::Text,
        });

        assert!(validate_page_endpoint_draft(&draft, &[draft.clone()]).is_empty());
    }
}
