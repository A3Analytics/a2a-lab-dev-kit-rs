use std::collections::HashMap;

use prost::Message;
use prost_reflect::{DynamicMessage, ReflectMessage};
use serde_json::json;

use crate::sila::consumer::codec::{self, Uploads, decode_fields, encode_fields};
use crate::sila::consumer::compile::{framework_pool, install_feature};
use crate::sila::consumer::fdl::parse_feature;
const SAMPLE: &str = r#"
<Feature SiLA2Version="1.0" FeatureVersion="1.0" Originator="org.a2alab" Category="tests">
  <Identifier>LabSample</Identifier>
  <DisplayName>Lab Sample</DisplayName>
  <Description>Sample</Description>
  <Command>
    <Identifier>Echo</Identifier>
    <DisplayName>Echo</DisplayName>
    <Description>Echo text</Description>
    <Observable>No</Observable>
    <Parameter>
      <Identifier>Message</Identifier>
      <DisplayName>Message</DisplayName>
      <Description>Text</Description>
      <DataType><Basic>String</Basic></DataType>
    </Parameter>
    <Response>
      <Identifier>Message</Identifier>
      <DisplayName>Message</DisplayName>
      <Description>Text</Description>
      <DataType><Basic>String</Basic></DataType>
    </Response>
  </Command>
  <Property>
    <Identifier>Flag</Identifier>
    <DisplayName>Flag</DisplayName>
    <Description>A flag</Description>
    <Observable>No</Observable>
    <DataType><Basic>Boolean</Basic></DataType>
  </Property>
  <Metadata>
    <Identifier>Token</Identifier>
    <DisplayName>Token</DisplayName>
    <Description>A token</Description>
    <DataType><Basic>String</Basic></DataType>
  </Metadata>
  <Command>
    <Identifier>Stamp</Identifier>
    <DisplayName>Stamp</DisplayName>
    <Description>Echo a date</Description>
    <Observable>No</Observable>
    <Parameter>
      <Identifier>When</Identifier>
      <DisplayName>When</DisplayName>
      <Description>Date</Description>
      <DataType><Basic>Date</Basic></DataType>
    </Parameter>
    <Response>
      <Identifier>When</Identifier>
      <DisplayName>When</DisplayName>
      <Description>Date</Description>
      <DataType><Basic>Date</Basic></DataType>
    </Response>
  </Command>
</Feature>
"#;

#[test]
fn parses_service_package_and_command_shape() {
    let feature = parse_feature(include_str!("../standard/SiLAService.sila.xml")).unwrap();
    assert_eq!(feature.fqi(), "org.silastandard/core/SiLAService/v1");
    assert_eq!(
        feature.package(),
        "sila2.org.silastandard.core.silaservice.v1"
    );
    let command = feature.command("GetFeatureDefinition").unwrap();
    assert!(!command.observable);
    assert_eq!(command.parameters[0].identifier, "FeatureIdentifier");
}

#[test]
fn compiles_property_and_command_methods() {
    let feature = parse_feature(SAMPLE).unwrap();
    let mut pool = framework_pool().unwrap().clone();
    install_feature(&mut pool, &feature).unwrap();
    let service = pool
        .get_service_by_name("sila2.org.a2alab.tests.labsample.v1.LabSample")
        .unwrap();
    let names: Vec<_> = service
        .methods()
        .map(|method| method.name().to_owned())
        .collect();
    assert!(names.contains(&"Echo".to_owned()));
    assert!(names.contains(&"Get_Flag".to_owned()));
    assert!(names.contains(&"Get_FCPAffectedByMetadata_Token".to_owned()));
}

#[test]
fn string_and_date_values_round_trip() {
    let feature = parse_feature(SAMPLE).unwrap();
    let mut pool = framework_pool().unwrap().clone();
    install_feature(&mut pool, &feature).unwrap();
    let elements = feature.command("Echo").unwrap().parameters.clone();
    let mut values = HashMap::new();
    values.insert("Message".to_owned(), json!("hello"));
    let encoded = encode_fields(
        &pool,
        &feature,
        "sila2.org.a2alab.tests.labsample.v1.Echo_Parameters",
        &elements,
        &values,
        &Uploads::default(),
        "",
    )
    .unwrap();
    let decoded = decode_fields(&feature, &elements, &encoded).unwrap();
    assert_eq!(
        decoded.get("Message").and_then(|value| value.as_str()),
        Some("hello")
    );

    let date_elements = feature.command("Stamp").unwrap().parameters.clone();
    let mut values = HashMap::new();
    values.insert("When".to_owned(), json!("2022-08-05+02:00"));
    let encoded = encode_fields(
        &pool,
        &feature,
        "sila2.org.a2alab.tests.labsample.v1.Stamp_Parameters",
        &date_elements,
        &values,
        &Uploads::default(),
        "",
    )
    .unwrap();
    let decoded = decode_fields(&feature, &date_elements, &encoded).unwrap();
    assert_eq!(
        decoded.get("When").and_then(|value| value.as_str()),
        Some("2022-08-05+02:00")
    );
}

#[test]
fn metadata_header_uses_the_part_b_name() {
    let header = format!(
        "sila-{}-bin",
        "org.silastandard/test/MetadataProvider/v1/Metadata/StringMetadata".replace('/', "-")
    )
    .to_ascii_lowercase();
    assert_eq!(
        header,
        "sila-org.silastandard-test-metadataprovider-v1-metadata-stringmetadata-bin"
    );
}

#[test]
fn aborted_status_decodes_defined_execution_error() {
    let pool = framework_pool().unwrap();
    let descriptor = pool
        .get_message_by_name("sila2.org.silastandard.SiLAError")
        .unwrap();
    let defined = pool
        .get_message_by_name("sila2.org.silastandard.DefinedExecutionError")
        .unwrap();
    let mut error = DynamicMessage::new(defined);
    let id = error
        .descriptor()
        .get_field_by_name("errorIdentifier")
        .unwrap();
    error.set_field(
        &id,
        prost_reflect::Value::String(
            "org.a2alab/tests/LabSample/v1/DefinedExecutionError/Rejected".to_owned(),
        ),
    );
    let message_field = error.descriptor().get_field_by_name("message").unwrap();
    error.set_field(
        &message_field,
        prost_reflect::Value::String("nope".to_owned()),
    );
    let mut wrapper = DynamicMessage::new(descriptor);
    let field = wrapper
        .descriptor()
        .get_field_by_name("definedExecutionError")
        .unwrap();
    wrapper.set_field(&field, prost_reflect::Value::Message(error));
    let failure = codec::sila_error_from_status(
        pool,
        &tonic::Status::aborted(codec::encode_base64(&wrapper.encode_to_vec())),
    )
    .unwrap();
    assert_eq!(failure.kind, "defined_execution");
    assert_eq!(failure.message, "nope");
}

#[test]
fn binary_transfer_starts_above_two_mebibytes() {
    assert_eq!(codec::BINARY_LIMIT, 2 * 1024 * 1024);
}
