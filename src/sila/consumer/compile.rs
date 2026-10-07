//! Compile a Feature model into protobuf descriptors.

use std::path::PathBuf;
use std::sync::OnceLock;

use prost_reflect::DescriptorPool;
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, MethodDescriptorProto,
    ServiceDescriptorProto,
};

use crate::error::A2aLabError;
use crate::sila::consumer::model::{Basic, Element, FeatureModel, SilaType};

const FRAMEWORK_FILE: &str = "SiLAFramework.proto";

/// Framework and binary-transfer descriptors shared by every session.
pub(crate) fn framework_pool() -> Result<&'static DescriptorPool, A2aLabError> {
    static POOL: OnceLock<DescriptorPool> = OnceLock::new();
    if let Some(pool) = POOL.get() {
        return Ok(pool);
    }
    let built = build_framework()?;
    Ok(POOL.get_or_init(|| built))
}

fn build_framework() -> Result<DescriptorPool, A2aLabError> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto/sila");
    compile_dir(&directory)
}

fn compile_dir(directory: &std::path::Path) -> Result<DescriptorPool, A2aLabError> {
    let set = protox::compile(
        [
            FRAMEWORK_FILE,
            "SiLABinaryTransfer.proto",
            "SiLACloudConnector.proto",
        ],
        [directory],
    )
    .map_err(|error| A2aLabError::protocol(error.to_string()))?;
    DescriptorPool::from_file_descriptor_set(set)
        .map_err(|error| A2aLabError::protocol(error.to_string()))
}

/// Adds one feature to `pool` and returns its protobuf package.
pub(crate) fn install_feature(
    pool: &mut DescriptorPool,
    feature: &FeatureModel,
) -> Result<(), A2aLabError> {
    let file = feature_file(feature)?;
    pool.add_file_descriptor_proto(file)
        .map_err(|error| A2aLabError::protocol(error.to_string()))?;
    Ok(())
}

fn feature_file(feature: &FeatureModel) -> Result<FileDescriptorProto, A2aLabError> {
    let package = feature.package();
    let mut messages = Vec::new();
    let mut methods = Vec::new();
    for data_type in &feature.data_types {
        messages.push(custom_type(
            &package,
            &data_type.identifier,
            &data_type.data_type,
        )?);
    }
    for command in &feature.commands {
        command_messages(&package, command, &mut messages)?;
        command_methods(command, &package, &mut methods);
    }
    for property in &feature.properties {
        property_messages(&package, property, &mut messages)?;
        methods.push(property_method(property, &package));
    }
    for metadata in &feature.metadata {
        metadata_messages(&package, metadata, &mut messages)?;
        methods.push(metadata_method(metadata, &package));
    }
    Ok(FileDescriptorProto {
        name: Some(format!("{}.proto", feature.identifier)),
        package: Some(package),
        syntax: Some("proto3".to_owned()),
        dependency: vec![FRAMEWORK_FILE.to_owned()],
        message_type: messages,
        service: vec![ServiceDescriptorProto {
            name: Some(feature.identifier.clone()),
            method: methods,
            ..Default::default()
        }],
        ..Default::default()
    })
}

fn command_messages(
    package: &str,
    command: &crate::sila::consumer::model::CommandModel,
    messages: &mut Vec<DescriptorProto>,
) -> Result<(), A2aLabError> {
    let name = &command.identifier;
    messages.push(element_message(
        package,
        &format!("{name}_Parameters"),
        &command.parameters,
    )?);
    messages.push(element_message(
        package,
        &format!("{name}_Responses"),
        &command.responses,
    )?);
    if command.observable && !command.intermediate.is_empty() {
        messages.push(element_message(
            package,
            &format!("{name}_IntermediateResponses"),
            &command.intermediate,
        )?);
    }
    Ok(())
}

fn command_methods(
    command: &crate::sila::consumer::model::CommandModel,
    package: &str,
    methods: &mut Vec<MethodDescriptorProto>,
) {
    let name = &command.identifier;
    if !command.observable {
        methods.push(method(
            name,
            &qualified(package, &format!("{name}_Parameters")),
            &qualified(package, &format!("{name}_Responses")),
            false,
        ));
        return;
    }
    methods.push(method(
        name,
        &qualified(package, &format!("{name}_Parameters")),
        "sila2.org.silastandard.CommandConfirmation",
        false,
    ));
    methods.push(method(
        &format!("{name}_Info"),
        "sila2.org.silastandard.CommandExecutionUUID",
        "sila2.org.silastandard.ExecutionInfo",
        true,
    ));
    if !command.intermediate.is_empty() {
        methods.push(method(
            &format!("{name}_Intermediate"),
            "sila2.org.silastandard.CommandExecutionUUID",
            &qualified(package, &format!("{name}_IntermediateResponses")),
            true,
        ));
    }
    methods.push(method(
        &format!("{name}_Result"),
        "sila2.org.silastandard.CommandExecutionUUID",
        &qualified(package, &format!("{name}_Responses")),
        false,
    ));
}

fn property_messages(
    package: &str,
    property: &crate::sila::consumer::model::PropertyModel,
    messages: &mut Vec<DescriptorProto>,
) -> Result<(), A2aLabError> {
    let name = property_rpc(property);
    messages.push(empty_message(&format!("{name}_Parameters")));
    messages.push(single_message(
        package,
        &format!("{name}_Responses"),
        &property.identifier,
        &property.data_type,
    )?);
    Ok(())
}

fn property_method(
    property: &crate::sila::consumer::model::PropertyModel,
    package: &str,
) -> MethodDescriptorProto {
    let name = property_rpc(property);
    method(
        &name,
        &qualified(package, &format!("{name}_Parameters")),
        &qualified(package, &format!("{name}_Responses")),
        property.observable,
    )
}

fn metadata_messages(
    package: &str,
    metadata: &crate::sila::consumer::model::MetadataModel,
    messages: &mut Vec<DescriptorProto>,
) -> Result<(), A2aLabError> {
    let affected = format!("Get_FCPAffectedByMetadata_{}", metadata.identifier);
    messages.push(empty_message(&format!("{affected}_Parameters")));
    messages.push(single_message(
        package,
        &format!("{affected}_Responses"),
        "AffectedCalls",
        &SilaType::List(Box::new(SilaType::Basic(Basic::String))),
    )?);
    messages.push(single_message(
        package,
        &format!("Metadata_{}", metadata.identifier),
        &metadata.identifier,
        &metadata.data_type,
    )?);
    Ok(())
}

fn metadata_method(
    metadata: &crate::sila::consumer::model::MetadataModel,
    package: &str,
) -> MethodDescriptorProto {
    let name = format!("Get_FCPAffectedByMetadata_{}", metadata.identifier);
    method(
        &name,
        &qualified(package, &format!("{name}_Parameters")),
        &qualified(package, &format!("{name}_Responses")),
        false,
    )
}

fn custom_type(
    package: &str,
    identifier: &str,
    data_type: &SilaType,
) -> Result<DescriptorProto, A2aLabError> {
    single_message(
        package,
        &format!("DataType_{identifier}"),
        identifier,
        data_type,
    )
}

fn element_message(
    package: &str,
    name: &str,
    elements: &[Element],
) -> Result<DescriptorProto, A2aLabError> {
    let qualified_name = qualified(package, name);
    build_message(name, &qualified_name, package, elements)
}

fn single_message(
    package: &str,
    name: &str,
    field: &str,
    data_type: &SilaType,
) -> Result<DescriptorProto, A2aLabError> {
    element_message(
        package,
        name,
        &[Element {
            identifier: field.to_owned(),
            data_type: data_type.clone(),
        }],
    )
}

fn build_message(
    name: &str,
    qualified_name: &str,
    package: &str,
    elements: &[Element],
) -> Result<DescriptorProto, A2aLabError> {
    let mut fields = Vec::new();
    let mut nested = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        let number =
            i32::try_from(index + 1).map_err(|_| A2aLabError::protocol("too many SiLA fields"))?;
        let mapped = map_field(
            package,
            &element.identifier,
            &element.data_type,
            qualified_name,
            number,
        )?;
        fields.push(mapped.field);
        nested.extend(mapped.nested);
    }
    Ok(DescriptorProto {
        name: Some(name.to_owned()),
        field: fields,
        nested_type: nested,
        ..Default::default()
    })
}

struct Mapped {
    field: FieldDescriptorProto,
    nested: Vec<DescriptorProto>,
}

fn map_field(
    package: &str,
    identifier: &str,
    data_type: &SilaType,
    parent: &str,
    number: i32,
) -> Result<Mapped, A2aLabError> {
    let mut nested = Vec::new();
    let (type_name, repeated) = resolve(package, identifier, data_type, parent, &mut nested)?;
    Ok(Mapped {
        field: message_field(identifier, number, &type_name, repeated),
        nested,
    })
}

fn resolve(
    package: &str,
    identifier: &str,
    data_type: &SilaType,
    parent: &str,
    nested: &mut Vec<DescriptorProto>,
) -> Result<(String, bool), A2aLabError> {
    match data_type.effective() {
        SilaType::Basic(basic) => Ok((framework_message(*basic).to_owned(), false)),
        SilaType::Reference(reference) => Ok((format!("{package}.DataType_{reference}"), false)),
        SilaType::Structure(elements) => {
            let name = format!("{identifier}_Struct");
            let qualified_name = format!("{parent}.{name}");
            nested.push(build_message(&name, &qualified_name, package, elements)?);
            Ok((qualified_name, false))
        }
        SilaType::List(item) => {
            let (name, _) = resolve(package, identifier, item, parent, nested)?;
            Ok((name, true))
        }
        SilaType::Constrained { .. } => Err(A2aLabError::protocol(
            "constraint did not reduce to a wire type",
        )),
    }
}

fn framework_message(basic: Basic) -> &'static str {
    match basic {
        Basic::String => "sila2.org.silastandard.String",
        Basic::Integer => "sila2.org.silastandard.Integer",
        Basic::Real => "sila2.org.silastandard.Real",
        Basic::Boolean => "sila2.org.silastandard.Boolean",
        Basic::Binary => "sila2.org.silastandard.Binary",
        Basic::Date => "sila2.org.silastandard.Date",
        Basic::Time => "sila2.org.silastandard.Time",
        Basic::Timestamp => "sila2.org.silastandard.Timestamp",
        Basic::Any => "sila2.org.silastandard.Any",
    }
}

fn message_field(name: &str, number: i32, type_name: &str, repeated: bool) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_owned()),
        number: Some(number),
        label: Some(if repeated {
            Label::Repeated as i32
        } else {
            Label::Optional as i32
        }),
        r#type: Some(Type::Message as i32),
        type_name: Some(format!(".{type_name}")),
        json_name: Some(name.to_owned()),
        ..Default::default()
    }
}

fn method(name: &str, input: &str, output: &str, server_streaming: bool) -> MethodDescriptorProto {
    MethodDescriptorProto {
        name: Some(name.to_owned()),
        input_type: Some(format!(".{input}")),
        output_type: Some(format!(".{output}")),
        client_streaming: Some(false),
        server_streaming: Some(server_streaming),
        ..Default::default()
    }
}

fn empty_message(name: &str) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.to_owned()),
        ..Default::default()
    }
}

fn property_rpc(property: &crate::sila::consumer::model::PropertyModel) -> String {
    if property.observable {
        format!("Subscribe_{}", property.identifier)
    } else {
        format!("Get_{}", property.identifier)
    }
}

fn qualified(package: &str, name: &str) -> String {
    format!("{package}.{name}")
}
