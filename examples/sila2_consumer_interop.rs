//! Exercise one running server and write a capability report.

use std::env;
use std::fs;
use std::time::Duration;

use a2a_lab_dev_kit::sila::{SilaDevice, SilaEndpoint, SilaSession, SilaTrust};
use a2a_lab_dev_kit::{
    GetTaskStatusRequest, JsonObject, ListTasksRequest, PageRequest, StartTaskRequest,
    TaskDefinition, TaskProvider, TaskRun, TaskState,
};
use serde_json::{Value, json};

const METADATA_STRING: &str = "org.silastandard/test/MetadataProvider/v1/Metadata/StringMetadata";
const METADATA_INTEGERS: &str =
    "org.silastandard/test/MetadataProvider/v1/Metadata/TwoIntegersMetadata";
const METADATA_BINARY: &str = "org.silastandard/test/BinaryTransferTest/v1/Metadata/String";
const METADATA_TOKEN: &str = "org.silastandard/core/AuthorizationService/v1/Metadata/AccessToken";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = env::var("SILA_PEER")?;
    let host = env::var("SILA_HOST")?;
    let port: u16 = env::var("SILA_PORT")?.parse()?;
    let server_uuid = env::var("SILA_UUID")?;
    let ca = fs::read(env::var("SILA_CA")?)?;
    let out = env::var("SILA_REPORT")?;
    let trust = SilaTrust::authority_pem(ca);
    let endpoint = SilaEndpoint {
        host: host.clone(),
        port,
        server_uuid: server_uuid.clone(),
    };
    let session = SilaSession::connect(&endpoint, &trust).await?;
    let mut capabilities = connection_capabilities(&session);
    capabilities.push(feature_xml(
        &session,
        a2a_lab_dev_kit::sila::SILA_SERVICE,
        "feature-definition",
    )?);
    capabilities.push(feature_xml(
        &session,
        "org.silastandard/test/MetadataProvider/v1",
        "metadata-feature-definition",
    )?);
    capabilities.push(affected(&session).await);
    if let Ok(plain_port) = env::var("SILA_PLAIN_PORT") {
        capabilities.push(unencrypted(&host, plain_port.parse()?, &server_uuid).await);
    }
    let cloud_port = env::var("SILA_CLOUD_PORT")
        .ok()
        .and_then(|port| port.parse().ok());
    let device = SilaDevice::new(session)?;
    let tasks = device.tasks();
    let definitions = all_tasks(&tasks).await?;
    capabilities.extend(exercise(&tasks, &definitions, &server_uuid).await);
    capabilities.push(discover(&trust, &server_uuid).await);
    if let Ok(port) = env::var("SILA_INITIATED_PORT") {
        let initiated_uuid = env::var("SILA_INITIATED_UUID")?;
        capabilities.push(
            server_initiated(&host, port.parse()?, &initiated_uuid, &trust, cloud_port).await,
        );
    }
    let failed = capabilities.iter().any(|item| item["status"] == "fail");
    let report = json!({
        "peer": peer,
        "host": host,
        "port": port,
        "server_uuid": server_uuid,
        "capabilities": capabilities,
    });
    fs::write(out, serde_json::to_string_pretty(&report)?)?;
    if failed {
        return Err("one or more SiLA capabilities failed".into());
    }
    Ok(())
}

fn connection_capabilities(session: &SilaSession) -> Vec<Value> {
    vec![
        pass(
            "direct-connection",
            "encrypted client-initiated connection with the SiLA2 certificate name",
        ),
        pass(
            "tls-identity",
            "server UUID matched the certificate session",
        ),
        pass(
            "sila-service",
            &format!("{} {}", session.server_name(), session.server_uuid()),
        ),
    ]
}

fn feature_xml(
    session: &SilaSession,
    feature: &str,
    id: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let definition = session.feature_definition(feature)?;
    Ok(if definition.contains("<Feature") {
        pass(id, feature)
    } else {
        fail(id, "XML did not contain a Feature")
    })
}

async fn all_tasks(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
) -> Result<Vec<TaskDefinition>, Box<dyn std::error::Error>> {
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = tasks
            .list_tasks(ListTasksRequest {
                page: PageRequest::new(cursor.clone(), 1000)?,
            })
            .await?;
        items.extend(page.items().iter().cloned());
        match page.next_cursor() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    Ok(items)
}

async fn discover(trust: &SilaTrust, server_uuid: &str) -> Value {
    match SilaSession::discover(
        a2a_lab_dev_kit::sila::SILA_SERVICE,
        trust,
        Duration::from_secs(3),
    )
    .await
    {
        Ok(server) if server.server_uuid().eq_ignore_ascii_case(server_uuid) => {
            pass("mdns-discovery", "browsed _sila._tcp.local.")
        }
        Ok(server) => fail(
            "mdns-discovery",
            &format!("browse returned {}", server.server_uuid()),
        ),
        Err(error) => fail("mdns-discovery", &error.to_string()),
    }
}

async fn unencrypted(host: &str, port: u16, server_uuid: &str) -> Value {
    let endpoint = SilaEndpoint {
        host: host.to_owned(),
        port,
        server_uuid: server_uuid.to_owned(),
    };
    match SilaSession::connect_unencrypted(&endpoint).await {
        Ok(session) if session.server_uuid().eq_ignore_ascii_case(server_uuid) => pass(
            "unencrypted-connection",
            "plaintext gateway reached SiLAService",
        ),
        Ok(session) => fail(
            "unencrypted-connection",
            &format!("gateway returned {}", session.server_uuid()),
        ),
        Err(error) => fail("unencrypted-connection", &error.to_string()),
    }
}

async fn affected(session: &SilaSession) -> Value {
    let feature = "org.silastandard/test/MetadataProvider/v1";
    let string_calls = session.affected_calls(feature, "StringMetadata").await;
    let integer_calls = session.affected_calls(feature, "TwoIntegersMetadata").await;
    match (string_calls, integer_calls) {
        (Ok(string_calls), Ok(integer_calls))
            if !string_calls.is_empty() && !integer_calls.is_empty() =>
        {
            pass(
                "metadata-affected-calls",
                &format!(
                    "string {} integers {}",
                    string_calls.len(),
                    integer_calls.len()
                ),
            )
        }
        (Ok(string_calls), Ok(integer_calls)) => fail(
            "metadata-affected-calls",
            &format!("string {string_calls:?} integers {integer_calls:?}"),
        ),
        (Err(error), _) | (_, Err(error)) => fail("metadata-affected-calls", &error.to_string()),
    }
}

async fn server_initiated(
    host: &str,
    port: u16,
    server_uuid: &str,
    trust: &SilaTrust,
    cloud_port: Option<u16>,
) -> Value {
    let Some(cloud_port) = cloud_port else {
        return fail(
            "server-initiated-connection",
            "cloud endpoint was not started",
        );
    };
    let cloud = tokio::spawn(async move { SilaSession::serve_cloud_endpoint(cloud_port).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let endpoint = SilaEndpoint {
        host: host.to_owned(),
        port,
        server_uuid: server_uuid.to_owned(),
    };
    let session = match SilaSession::connect(&endpoint, trust).await {
        Ok(session) => session,
        Err(error) => return fail("server-initiated-connection", &error.to_string()),
    };
    let device = match SilaDevice::new(session) {
        Ok(device) => device,
        Err(error) => return fail("server-initiated-connection", &error.to_string()),
    };
    let tasks = device.tasks();
    let definitions = match all_tasks(&tasks).await {
        Ok(definitions) => definitions,
        Err(error) => return fail("server-initiated-connection", &error.to_string()),
    };
    let input = r#"{"ClientName":"devkit","SiLAClientHost":"127.0.0.1","SiLAClientPort":50056,"Persist":false}"#;
    if let Err(item) = call(&tasks, &definitions, "command", "ConnectSiLAClient", input).await {
        return fail(
            "server-initiated-connection",
            item["detail"].as_str().unwrap_or("connect failed"),
        );
    }
    match cloud.await {
        Ok(Ok(name)) if !name.is_empty() => pass(
            "server-initiated-connection",
            &format!("read server name {name}"),
        ),
        Ok(Ok(_)) => fail("server-initiated-connection", "empty server name"),
        Ok(Err(error)) => fail("server-initiated-connection", &error.to_string()),
        Err(error) => fail("server-initiated-connection", &error.to_string()),
    }
}

async fn exercise(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
    server_uuid: &str,
) -> Vec<Value> {
    let mut results = Vec::new();
    results.extend(service_calls(tasks, definitions).await);
    results.push(basic_types(tasks, definitions).await);
    results.push(structures(tasks, definitions).await);
    results.push(lists(tasks, definitions).await);
    results.push(any_types(tasks, definitions).await);
    results.push(unobservable_commands(tasks, definitions).await);
    results.push(unobservable_properties(tasks, definitions).await);
    results.push(metadata(tasks, definitions).await);
    results.push(errors(tasks, definitions).await);
    results.extend(observable(tasks, definitions).await);
    results.push(observable_properties(tasks, definitions).await);
    results.push(cancel(tasks, definitions).await);
    results.push(binaries(tasks, definitions).await);
    results.push(authorization(tasks, definitions, server_uuid).await);
    results
}

async fn service_calls(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Vec<Value> {
    let mut results = vec![
        call(
            tasks,
            definitions,
            "command",
            "GetFeatureDefinition",
            r#"{"FeatureIdentifier":"org.silastandard/core/SiLAService/v1"}"#,
        )
        .await
        .map_or_else(
            |item| item,
            |_| pass("get-feature-definition", "SiLAService"),
        ),
    ];
    let renamed = call(
        tasks,
        definitions,
        "command",
        "SetServerName",
        r#"{"ServerName":"SiLA is Awesome"}"#,
    )
    .await;
    results.push(renamed.map_or_else(|item| item, |_| pass("set-server-name", "renamed")));
    for property in [
        "ServerName",
        "ServerType",
        "ServerUUID",
        "ServerDescription",
        "ServerVersion",
        "ServerVendorURL",
        "ImplementedFeatures",
    ] {
        let read = call(tasks, definitions, "property", property, "{}").await;
        results.push(read.map_or_else(|item| item, |_| pass(property, "read")));
    }
    results
}

async fn basic_types(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    let calls = [
        (
            "EchoStringValue",
            r#"{"StringValue":"SiLA2_Test_String_Value"}"#,
            "ReceivedValue",
            json!("SiLA2_Test_String_Value"),
        ),
        (
            "EchoIntegerValue",
            r#"{"IntegerValue":5124}"#,
            "ReceivedValue",
            json!(5124),
        ),
        (
            "EchoBooleanValue",
            r#"{"BooleanValue":true}"#,
            "ReceivedValue",
            json!(true),
        ),
        (
            "EchoDateValue",
            r#"{"DateValue":"2022-08-05+02:00"}"#,
            "ReceivedValue",
            Value::Null,
        ),
        (
            "EchoTimeValue",
            r#"{"TimeValue":"12:34:56.789+02:00"}"#,
            "ReceivedValue",
            Value::Null,
        ),
        (
            "EchoTimestampValue",
            r#"{"TimestampValue":"2022-08-05 12:34:56.789+02:00"}"#,
            "ReceivedValue",
            Value::Null,
        ),
    ];
    for (member, input, field, expected) in calls {
        if let Err(item) = expect_field(
            tasks,
            definitions,
            "command",
            member,
            input,
            field,
            &expected,
        )
        .await
        {
            return item;
        }
    }
    let real = call(
        tasks,
        definitions,
        "command",
        "EchoRealValue",
        r#"{"RealValue":3.1415926}"#,
    )
    .await;
    if let Err(item) = real.and_then(|run| {
        let text = field(&run, "ReceivedValue")
            .map(ToString::to_string)
            .unwrap_or_default();
        if text.starts_with("3.14") {
            Ok(run)
        } else {
            Err(fail("basic-data-types", &format!("real was {text}")))
        }
    }) {
        return item;
    }
    for property in [
        "StringValue",
        "IntegerValue",
        "RealValue",
        "BooleanValue",
        "DateValue",
        "TimeValue",
        "TimestampValue",
    ] {
        if let Err(item) = call(tasks, definitions, "property", property, "{}").await {
            return item;
        }
    }
    pass("basic-data-types", "echo and read")
}

async fn structures(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    let structure = r#"{"StructureValue":{"StringTypeValue":"SiLA2_Test_String_Value","IntegerTypeValue":5124,"RealTypeValue":3.1415926,"BooleanTypeValue":true,"BinaryTypeValue":"U2lMQTJfQmluYXJ5X1N0cmluZ19WYWx1ZQ==","DateTypeValue":"2022-08-05+02:00","TimeTypeValue":"12:34:56.789+02:00","TimestampTypeValue":"2022-08-05 12:34:56.789+02:00","AnyTypeValue":"SiLA2_Any_Type_String_Value"}}"#;
    if let Err(item) = expect_field(
        tasks,
        definitions,
        "command",
        "EchoStructureValue",
        structure,
        "ReceivedValues",
        &Value::Null,
    )
    .await
    {
        return item;
    }
    let deep = r#"{"DeepStructureValue":{"OuterStringTypeValue":"Outer_Test_String","OuterIntegerTypeValue":1111,"MiddleStructure":{"MiddleStringTypeValue":"Middle_Test_String","MiddleIntegerTypeValue":2222,"InnerStructure":{"InnerStringTypeValue":"Inner_Test_String","InnerIntegerTypeValue":3333}}}}"#;
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoDeepStructureValue",
        deep,
    )
    .await
    {
        return item;
    }
    for property in ["StructureValue", "DeepStructureValue"] {
        if let Err(item) = call(tasks, definitions, "property", property, "{}").await {
            return item;
        }
    }
    pass("structured-data", "structure and deep structure")
}

async fn lists(tasks: &a2a_lab_dev_kit::sila::SilaTasks, definitions: &[TaskDefinition]) -> Value {
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoStringList",
        r#"{"StringList":["SiLA 2","is","great"]}"#,
    )
    .await
    {
        return item;
    }
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoIntegerList",
        r#"{"IntegerList":[1,2,3]}"#,
    )
    .await
    {
        return item;
    }
    let structure_list = r#"{"StructureList":[{"StringTypeValue":"SiLA2_Test_String_Value_1","IntegerTypeValue":5124,"RealTypeValue":3.1415926,"BooleanTypeValue":true,"BinaryTypeValue":"QmluYXJ5X1N0cmluZ19WYWx1ZV8x","DateTypeValue":"2022-08-05+02:00","TimeTypeValue":"12:34:56.789+02:00","TimestampTypeValue":"2022-08-05 12:34:56.789+02:00","AnyTypeValue":"Any_Type_String_Value_1"}]}"#;
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoStructureList",
        structure_list,
    )
    .await
    {
        return item;
    }
    for property in [
        "EmptyStringList",
        "StringList",
        "IntegerList",
        "StructureList",
    ] {
        if let Err(item) = call(tasks, definitions, "property", property, "{}").await {
            return item;
        }
    }
    pass("list-data", "string, integer, and structure lists")
}

async fn any_types(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    let calls = [
        r#"{"AnyTypeValue":{"type":"Void"}}"#,
        r#"{"AnyTypeValue":{"type":"String","value":"SiLA_Any_type_of_String_type"}}"#,
        r#"{"AnyTypeValue":{"type":"Integer","value":5124}}"#,
        r#"{"AnyTypeValue":{"type":"Real","value":3.1415926}}"#,
        r#"{"AnyTypeValue":{"type":"Boolean","value":true}}"#,
        r#"{"AnyTypeValue":{"type":"Binary","value":"U2lMQV9BbnlfdHlwZV9vZl9CaW5hcnlfdHlwZQ=="}}"#,
        r#"{"AnyTypeValue":{"type":"Date","value":"2022-08-05+02:00"}}"#,
        r#"{"AnyTypeValue":{"type":"Time","value":"12:34:56.789+02:00"}}"#,
        r#"{"AnyTypeValue":{"type":"Timestamp","value":"2022-08-05 12:34:56.789+02:00"}}"#,
        r#"{"AnyTypeValue":{"type":"List","value":["SiLA 2","Any","Type","String","List"]}}"#,
        r#"{"AnyTypeValue":{"type":"Structure","value":{"StringTypeValue":"A String value","IntegerTypeValue":{"type":"Integer","value":83737665},"DateTypeValue":{"type":"Date","value":"2022-08-05+02:00"}}}}"#,
    ];
    for input in calls {
        if let Err(item) = call(tasks, definitions, "command", "SetAnyTypeValue", input).await {
            return item;
        }
    }
    for property in [
        "AnyTypeStringValue",
        "AnyTypeIntegerValue",
        "AnyTypeRealValue",
        "AnyTypeBooleanValue",
        "AnyTypeBinaryValue",
        "AnyTypeDateValue",
        "AnyTypeTimeValue",
        "AnyTypeTimestampValue",
        "AnyTypeListValue",
        "AnyTypeStructureValue",
    ] {
        if let Err(item) = call(tasks, definitions, "property", property, "{}").await {
            return item;
        }
    }
    pass("any-type", "set and read")
}

async fn unobservable_commands(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    let calls = [
        ("CommandWithoutParametersAndResponses", "{}"),
        ("ConvertIntegerToString", r#"{"Integer":12345}"#),
        ("JoinIntegerAndString", r#"{"Integer":123,"String":"abc"}"#),
        ("SplitStringAfterFirstCharacter", r#"{"String":""}"#),
        ("SplitStringAfterFirstCharacter", r#"{"String":"a"}"#),
        ("SplitStringAfterFirstCharacter", r#"{"String":"ab"}"#),
        ("SplitStringAfterFirstCharacter", r#"{"String":"abcde"}"#),
    ];
    for (member, input) in calls {
        if let Err(item) = call(tasks, definitions, "command", member, input).await {
            return item;
        }
    }
    pass("unobservable-command", "parameters and responses")
}

async fn unobservable_properties(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    for property in ["AnswerToEverything", "SecondsSince1970"] {
        if let Err(item) = call(tasks, definitions, "property", property, "{}").await {
            return item;
        }
    }
    pass("unobservable-property", "read")
}

async fn metadata(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    let string = format!(r#"{{"metadata":{{"{METADATA_STRING}":"abc"}}}}"#);
    if let Err(item) = expect_field(
        tasks,
        definitions,
        "command",
        "EchoStringMetadata",
        &string,
        "ReceivedStringMetadata",
        &json!("abc"),
    )
    .await
    {
        return item;
    }
    let unpacked = format!(
        r#"{{"metadata":{{"{METADATA_STRING}":"abc","{METADATA_INTEGERS}":{{"FirstInteger":123,"SecondInteger":456}}}}}}"#
    );
    if let Err(item) = expect_field(
        tasks,
        definitions,
        "command",
        "UnpackMetadata",
        &unpacked,
        "ReceivedString",
        &json!("abc"),
    )
    .await
    {
        return item;
    }
    if let Err(item) = call(
        tasks,
        definitions,
        "property",
        "ReceivedStringMetadata",
        &string,
    )
    .await
    {
        return item;
    }
    pass("metadata", "string and structure metadata")
}

async fn errors(tasks: &a2a_lab_dev_kit::sila::SilaTasks, definitions: &[TaskDefinition]) -> Value {
    for member in [
        "RaiseDefinedExecutionError",
        "RaiseUndefinedExecutionError",
        "RaiseDefinedExecutionErrorObservably",
        "RaiseUndefinedExecutionErrorObservably",
    ] {
        if let Err(item) = expect_error(tasks, definitions, "command", member, "{}").await {
            return item;
        }
    }
    for property in [
        "RaiseDefinedExecutionErrorOnGet",
        "RaiseUndefinedExecutionErrorOnGet",
    ] {
        if let Err(item) = expect_error(tasks, definitions, "property", property, "{}").await {
            return item;
        }
    }
    pass("error-classes", "defined and undefined execution errors")
}

async fn observable(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Vec<Value> {
    let Some(task) = find(definitions, "command", "Count") else {
        return vec![unsupported(
            "observable-command",
            "peer has no Count command",
        )];
    };
    let started = tasks
        .start(
            StartTaskRequest::new(
                task.id.clone(),
                JsonObject::parse(r#"{"N":5,"Delay":1}"#).unwrap(),
            )
            .immediate(),
        )
        .await;
    let Ok(run) = started else {
        return vec![fail(
            "observable-command",
            &started.unwrap_err().to_string(),
        )];
    };
    let finished = match until_terminal(tasks, run).await {
        Ok(run) => run,
        Err(item) => return vec![item],
    };
    if finished.state != TaskState::Completed {
        return vec![fail("observable-command", &format!("{:?}", finished.state))];
    }
    if finished.progress.is_none() {
        return vec![fail("observable-command", "Count reported no progress")];
    }
    if field(&finished, "IterationResponse") != Some(&json!(4)) {
        return vec![fail("observable-command", "Count did not finish at 4")];
    }
    let iterations = finished
        .result
        .as_ref()
        .and_then(|result| result.as_map().get("intermediate"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("CurrentIteration").and_then(Value::as_i64))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if iterations != [0, 1, 2, 3, 4] && iterations != [0, 1, 2, 3] {
        return vec![fail(
            "intermediate-responses",
            &format!("CurrentIteration was {iterations:?}"),
        )];
    }
    let intermediate = pass(
        "intermediate-responses",
        &format!("CurrentIteration {iterations:?}"),
    );
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoValueAfterDelay",
        r#"{"Value":3,"Delay":1}"#,
    )
    .await
    {
        return vec![item];
    }
    vec![
        pass("observable-command", "Count progress and delayed echo"),
        intermediate,
    ]
}

async fn observable_properties(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    if let Err(item) = expect_field(
        tasks,
        definitions,
        "property",
        "FixedValue",
        "{}",
        "value",
        &json!(42),
    )
    .await
    {
        return item;
    }
    let alternating = call(
        tasks,
        definitions,
        "property",
        "Alternating",
        r#"{"count":3}"#,
    )
    .await;
    match alternating {
        Ok(run)
            if run
                .result
                .as_ref()
                .and_then(|result| result.as_map().get("values"))
                .and_then(Value::as_array)
                .is_some_and(|values| values.len() == 3) =>
        {
            pass(
                "observable-property-subscription",
                "FixedValue and Alternating",
            )
        }
        Ok(_) => fail(
            "observable-property-subscription",
            "Alternating did not return three values",
        ),
        Err(item) => item,
    }
}

async fn cancel(tasks: &a2a_lab_dev_kit::sila::SilaTasks, definitions: &[TaskDefinition]) -> Value {
    let Some(task) = find(definitions, "command", "Count") else {
        return unsupported("cancellation", "peer has no Count command");
    };
    let started = tasks
        .start(
            StartTaskRequest::new(
                task.id.clone(),
                JsonObject::parse(r#"{"N":30,"Delay":1}"#).unwrap(),
            )
            .immediate(),
        )
        .await;
    let Ok(run) = started else {
        return fail("cancellation", &started.unwrap_err().to_string());
    };
    match tasks.cancel(GetTaskStatusRequest { id: run.id }).await {
        Ok(run)
            if matches!(
                run.state,
                TaskState::Canceled | TaskState::Failed | TaskState::Completed
            ) =>
        {
            pass("cancellation", "CancelController accepted the execution")
        }
        Ok(run) => fail("cancellation", &format!("{:?}", run.state)),
        Err(error) => unsupported(
            "cancellation",
            &format!("official server did not serve CancelController: {error}"),
        ),
    }
}

async fn binaries(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
) -> Value {
    let small = base64_encode(b"abc");
    let large = base64_encode(&"abc".repeat(1_000_000).into_bytes());
    if let Err(item) = call(tasks, definitions, "property", "BinaryValueDirectly", "{}").await {
        return item;
    }
    if let Err(item) = call(tasks, definitions, "property", "BinaryValueDownload", "{}").await {
        return item;
    }
    for encoded in [&small, &large] {
        let input = format!(r#"{{"BinaryValue":"{encoded}"}}"#);
        if let Err(item) = call(tasks, definitions, "command", "EchoBinaryValue", &input).await {
            return item;
        }
    }
    let listed =
        format!(r#"{{"Binaries":["{small}","{large}","U2lMQTJfVGVzdF9TdHJpbmdfVmFsdWU="]}}"#);
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoBinariesObservably",
        &listed,
    )
    .await
    {
        return item;
    }
    let with_metadata =
        format!(r#"{{"Binary":"{small}","metadata":{{"{METADATA_BINARY}":"abc"}}}}"#);
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoBinaryAndMetadataString",
        &with_metadata,
    )
    .await
    {
        return item;
    }
    let large_metadata =
        format!(r#"{{"Binary":"{large}","metadata":{{"{METADATA_BINARY}":"def"}}}}"#);
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "EchoBinaryAndMetadataString",
        &large_metadata,
    )
    .await
    {
        return item;
    }
    pass("binary-transfer", "inline, upload, and download")
}

async fn authorization(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
    server_uuid: &str,
) -> Value {
    let login = format!(
        r#"{{"UserIdentification":"test","Password":"test","RequestedServer":"{}","RequestedFeatures":["org.silastandard/test/AuthenticationTest/v1"]}}"#,
        server_uuid.to_ascii_lowercase()
    );
    let run = match call(tasks, definitions, "command", "Login", &login).await {
        Ok(run) => run,
        Err(item) => return item,
    };
    let Some(token) = field(&run, "AccessToken").and_then(Value::as_str) else {
        return fail("authorization", "Login did not return an access token");
    };
    let requires = format!(r#"{{"metadata":{{"{METADATA_TOKEN}":"{token}"}}}}"#);
    if let Err(item) = call(tasks, definitions, "command", "RequiresToken", &requires).await {
        return item;
    }
    let upload = format!(
        r#"{{"BinaryToUpload":"{}","metadata":{{"{METADATA_TOKEN}":"{token}"}}}}"#,
        base64_encode(b"abc")
    );
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "RequiresTokenForBinaryUpload",
        &upload,
    )
    .await
    {
        return item;
    }
    let large = base64_encode(&"abc".repeat(1_000_000).into_bytes());
    let large_upload =
        format!(r#"{{"BinaryToUpload":"{large}","metadata":{{"{METADATA_TOKEN}":"{token}"}}}}"#);
    if let Err(item) = call(
        tasks,
        definitions,
        "command",
        "RequiresTokenForBinaryUpload",
        &large_upload,
    )
    .await
    {
        return item;
    }
    let logout = format!(r#"{{"AccessToken":"{token}"}}"#);
    if let Err(item) = call(tasks, definitions, "command", "Logout", &logout).await {
        return item;
    }
    pass("authorization", "login, token metadata, logout")
}

async fn expect_field(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
    kind: &str,
    member: &str,
    input: &str,
    name: &str,
    expected: &Value,
) -> Result<TaskRun, Value> {
    let run = call(tasks, definitions, kind, member, input).await?;
    if expected.is_null() || field(&run, name) == Some(expected) {
        Ok(run)
    } else {
        Err(fail(member, &format!("{name} was {:?}", field(&run, name))))
    }
}

async fn expect_error(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
    kind: &str,
    member: &str,
    input: &str,
) -> Result<TaskRun, Value> {
    let Some(task) = find(definitions, kind, member) else {
        return Err(unsupported(member, "peer does not implement this member"));
    };
    match tasks
        .start(StartTaskRequest::new(
            task.id.clone(),
            JsonObject::parse(input).unwrap(),
        ))
        .await
    {
        Ok(run) if run.error_kind.is_some() => Ok(run),
        Ok(run) => match until_terminal(tasks, run).await {
            Ok(run) if run.error_kind.is_some() => Ok(run),
            Ok(run) => Err(fail(member, &format!("{:?}", run.state))),
            Err(item) => Err(item),
        },
        Err(error) => Err(fail(member, &error.to_string())),
    }
}

async fn call(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    definitions: &[TaskDefinition],
    kind: &str,
    member: &str,
    input: &str,
) -> Result<TaskRun, Value> {
    let Some(task) = find(definitions, kind, member) else {
        return Err(unsupported(member, "peer does not implement this member"));
    };
    let started = tasks
        .start(StartTaskRequest::new(
            task.id.clone(),
            JsonObject::parse(input).unwrap(),
        ))
        .await
        .map_err(|error| fail(member, &error.to_string()))?;
    let run = until_terminal(tasks, started).await?;
    if run.state == TaskState::Completed {
        Ok(run)
    } else {
        Err(fail(
            member,
            &format!("{:?} {}", run.state, run.message.unwrap_or_default()),
        ))
    }
}

async fn until_terminal(
    tasks: &a2a_lab_dev_kit::sila::SilaTasks,
    mut run: TaskRun,
) -> Result<TaskRun, Value> {
    for _ in 0..120 {
        if run.state.is_terminal() {
            return Ok(run);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        run = tasks
            .status(GetTaskStatusRequest { id: run.id.clone() })
            .await
            .map_err(|error| fail("status", &error.to_string()))?;
    }
    Err(fail("status", "run did not finish"))
}

fn field<'a>(run: &'a TaskRun, name: &str) -> Option<&'a Value> {
    run.result
        .as_ref()
        .and_then(|object| object.as_map().get(name))
}

fn find<'a>(
    definitions: &'a [TaskDefinition],
    kind: &str,
    member: &str,
) -> Option<&'a TaskDefinition> {
    definitions
        .iter()
        .find(|task| task.id.as_str().ends_with(&format!(":{kind}:{member}")))
}

fn pass(id: &str, detail: &str) -> Value {
    json!({"id": id, "status": "pass", "detail": detail})
}

fn fail(id: &str, detail: &str) -> Value {
    json!({"id": id, "status": "fail", "detail": detail})
}

fn unsupported(id: &str, detail: &str) -> Value {
    json!({"id": id, "status": "unsupported", "detail": detail})
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    let (chunks, rest) = bytes.as_chunks::<3>();
    for chunk in chunks {
        let value = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 6) & 63) as usize] as char);
        encoded.push(TABLE[(value & 63) as usize] as char);
    }
    if rest.len() == 1 {
        let value = u32::from(rest[0]) << 16;
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push('=');
        encoded.push('=');
    } else if rest.len() == 2 {
        let value = (u32::from(rest[0]) << 16) | (u32::from(rest[1]) << 8);
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 6) & 63) as usize] as char);
        encoded.push('=');
    }
    encoded
}
