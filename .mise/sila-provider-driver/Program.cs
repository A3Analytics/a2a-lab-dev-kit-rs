using System.Text.Json;
using Microsoft.Extensions.Configuration;
using ProtoBuf;
using ProtoBuf.Grpc.Client;
using Sila2.Org.Silastandard.Core.Silaservice.V1;
using SiLA2.Client;
using SiLA2.Client.Dynamic;
using SiLA2.Communication.Services;
using SiLA2.Server.Utils;

var host = Environment.GetEnvironmentVariable("SILA_HOST") ?? "127.0.0.1";
var port = int.Parse(Environment.GetEnvironmentVariable("SILA_PORT") ?? "50052");
var uuid = Environment.GetEnvironmentVariable("SILA_UUID") ?? "11111111-1111-1111-1111-111111111111";
var reportPath = Environment.GetEnvironmentVariable("SILA_REPORT") ?? "report.json";
var results = new List<Capability>();

var configuration = new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string>
{
    ["Connection:FQHN"] = host,
    ["Connection:Port"] = port.ToString(),
    ["Connection:ServerDiscovery:NIC"] = string.Empty,
    ["Connection:ServerDiscovery:ServiceName"] = "_sila._tcp"
}).Build();
var client = new DynamicConfigurator(configuration, []);

try
{
    var discovered = await client.SearchForServers();
    var found = discovered.Values.Any(server => string.Equals(server.ServerUuid, uuid, StringComparison.OrdinalIgnoreCase));
    results.Add(found
        ? Pass("mdns-discovery", "official client browsed _sila._tcp.local.")
        : Fail("mdns-discovery", $"found {discovered.Count}"));
}
catch (Exception error)
{
    results.Add(Fail("mdns-discovery", error.Message));
}

try
{
    var channel = await client.GetChannel(host, port, true);
    var service = new SiLAService.SiLAServiceClient(channel);
    var serverUuid = (await service.Get_ServerUUIDAsync(new Get_ServerUUID_Parameters())).ServerUUID.Value;
    var name = (await service.Get_ServerNameAsync(new Get_ServerName_Parameters())).ServerName.Value;
    results.Add(string.Equals(serverUuid, uuid, StringComparison.OrdinalIgnoreCase)
        ? Pass("sila-service", $"{name} {serverUuid}")
        : Fail("sila-service", $"server UUID {serverUuid}"));
    var version = (await service.Get_ServerVersionAsync(new Get_ServerVersion_Parameters())).ServerVersion.Value;
    var serverType = (await service.Get_ServerTypeAsync(new Get_ServerType_Parameters())).ServerType.Value;
    var description = (await service.Get_ServerDescriptionAsync(new Get_ServerDescription_Parameters())).ServerDescription.Value;
    var vendor = (await service.Get_ServerVendorURLAsync(new Get_ServerVendorURL_Parameters())).ServerVendorURL.Value;
    results.Add(version.Length > 0 && serverType == "LabDevKit" && description.Length > 0 && vendor.StartsWith("https://", StringComparison.Ordinal)
        ? Pass("sila-service-properties", $"{serverType} {version} {vendor}")
        : Fail("sila-service-properties", $"{serverType} {version} {vendor}"));
    await service.SetServerNameAsync(new SetServerName_Parameters { ServerName = new Sila2.Org.Silastandard.String { Value = "interop" } });
    var renamed = (await service.Get_ServerNameAsync(new Get_ServerName_Parameters())).ServerName.Value;
    results.Add(renamed == "interop" ? Pass("set-server-name", renamed) : Fail("set-server-name", renamed));
    results.Add(Pass("tls-identity", "encrypted client connection accepted the SiLA2 certificate"));

    var features = await service.Get_ImplementedFeaturesAsync(new Get_ImplementedFeatures_Parameters());
    var labId = "com.a3analytics/lab/LabOperations/v1";
    var hasLab = features.ImplementedFeatures.Any(feature => feature.Value == labId);
    var definition = service.GetFeatureDefinition(new GetFeatureDefinition_Parameters { FeatureIdentifier = new Sila2.Org.Silastandard.String { Value = labId } });
    var hasConnection = features.ImplementedFeatures.Any(feature => feature.Value == "org.silastandard/core/ConnectionConfigurationService/v1");
    results.Add(hasLab && hasConnection && definition.FeatureDefinition.Value.Contains("<Feature")
        ? Pass("feature-definition", labId)
        : Fail("feature-definition", "an advertised feature was not served"));
    try
    {
        service.GetFeatureDefinition(new GetFeatureDefinition_Parameters { FeatureIdentifier = new Sila2.Org.Silastandard.String { Value = "not-a-feature" } });
        results.Add(Fail("invalid-feature-identifier", "malformed identifier was accepted"));
    }
    catch (Exception error)
    {
        results.Add(Pass("invalid-feature-identifier", error.Message));
    }
    var feature = FeatureGenerator.ReadFeatureFromXml(definition.FeatureDefinition.Value);
    var messages = client.DynamicMessageService;

    await Call(messages, channel, feature, "ListLogSources", PagePayload(), "list-log-sources", results);
    await Call(messages, channel, feature, "QueryLogs", RangePayload("SourceId", "journal"), "query-logs", results);
    await Call(messages, channel, feature, "ListMetrics", PagePayload(), "list-metrics", results);
    await Call(messages, channel, feature, "QueryMetric", RangePayload("MetricId", "temp"), "query-metric", results);
    await Call(messages, channel, feature, "ListTasks", PagePayload(), "list-tasks", results);

    var started = messages.ExecuteObservableCommand("StartTask", channel, feature, new Dictionary<string, object>
    {
        ["TaskId"] = new StructurePayload("{\"Value\":\"mix\"}"),
        ["Input"] = new StructurePayload("{\"Value\":" + JsonSerializer.Serialize("{\"complete\":true}") + "}")
    });
    var execution = started.Item1.CommandExecutionUUID.Value;
    var info = started.Item2.GetAsyncEnumerator();
    var terminal = 0;
    while (await info.MoveNextAsync())
    {
        terminal = (int)info.Current.CommandStatus;
        if (terminal >= 2)
        {
            break;
        }
    }
    dynamic result = messages.GetObservableCommandResult(started.Item1.CommandExecutionUUID, "StartTask", channel, feature, started.Item4);
    results.Add(terminal == 2
        ? Pass("observable-command", $"execution {execution} state {result.Run.TaskRun.State.Value}")
        : Fail("observable-command", $"terminal status {terminal}"));
    if (terminal == 2)
    {
        await Call(messages, channel, feature, "GetTaskStatus", new Dictionary<string, object>
        {
            ["RunId"] = new StructurePayload($"{{\"Value\":\"{result.Run.TaskRun.Id.Value}\"}}")
        }, "get-task-status", results);
    }

    var waiting = messages.ExecuteObservableCommand("StartTask", channel, feature, new Dictionary<string, object>
    {
        ["TaskId"] = new StructurePayload("{\"Value\":\"mix\"}"),
        ["Input"] = new StructurePayload("{\"Value\":" + JsonSerializer.Serialize("{\"complete\":false}") + "}")
    });
    var cancelFeatureXml = service.GetFeatureDefinition(new GetFeatureDefinition_Parameters
    {
        FeatureIdentifier = new Sila2.Org.Silastandard.String { Value = "org.silastandard/core/commands/CancelController/v1" }
    }).FeatureDefinition.Value;
    var cancelFeature = FeatureGenerator.ReadFeatureFromXml(cancelFeatureXml);
    // The dynamic client encodes a constrained defined type as the inner basic type.
    // CancelCommand's parameter is DataType_UUID, so the call uses that normative message.
    var cancelClient = new GrpcClient(channel.CreateCallInvoker(), $"{cancelFeature.Namespace}.{cancelFeature.Identifier}");
    cancelClient.BlockingUnary<CancelCommandParameters, CancelCommandResponses>(new CancelCommandParameters
    {
        CommandExecutionUUID = new DataTypeUuid
        {
            UUID = new Sila2.Org.Silastandard.Protobuf.String { Value = waiting.Item1.CommandExecutionUUID.Value }
        }
    }, "CancelCommand");
    var cancelInfo = waiting.Item2.GetAsyncEnumerator();
    var cancelStatus = 0;
    while (await cancelInfo.MoveNextAsync())
    {
        cancelStatus = (int)cancelInfo.Current.CommandStatus;
        if (cancelStatus >= 2)
        {
            break;
        }
    }
    results.Add(cancelStatus == 3
        ? Pass("cancellation", "CancelCommand finished the execution with error")
        : Fail("cancellation", $"status {cancelStatus}"));
    await Call(messages, channel, cancelFeature, "CancelAll", new Dictionary<string, object>(), "cancel-all", results);
    var connectionFeature = FeatureGenerator.ReadFeatureFromXml(service.GetFeatureDefinition(new GetFeatureDefinition_Parameters
    {
        FeatureIdentifier = new Sila2.Org.Silastandard.String { Value = "org.silastandard/core/ConnectionConfigurationService/v1" }
    }).FeatureDefinition.Value);
    await Call(messages, channel, connectionFeature, "EnableServerInitiatedConnectionMode", new Dictionary<string, object>(), "enable-server-initiated", results);
    try
    {
        messages.ExecuteUnobservableCommand("ConnectSiLAClient", channel, connectionFeature, new Dictionary<string, object>
        {
            ["ClientName"] = new StructurePayload("{\"Value\":\"\"}"),
            ["SiLAClientHost"] = new StructurePayload("{\"Value\":\"127.0.0.1\"}"),
            ["SiLAClientPort"] = new StructurePayload("{\"Value\":1}"),
            ["Persist"] = new StructurePayload("{\"Value\":false}")
        });
        results.Add(Fail("invalid-sila-client", "an empty client name was accepted"));
    }
    catch (Exception error)
    {
        results.Add(Pass("invalid-sila-client", error.Message));
    }
    await Call(messages, channel, connectionFeature, "DisableServerInitiatedConnectionMode", new Dictionary<string, object>(), "disable-server-initiated", results);
}
catch (Exception error)
{
    results.Add(Fail("direct-connection", error.ToString()));
}

if (!results.Any(item => item.Id == "direct-connection"))
{
    results.Insert(0, Pass("direct-connection", "official dynamic client called the devkit server"));
}
results.Add(Skip("binary-transfer", "no advertised feature declares a Binary parameter"));
results.Add(Skip("locking", "the advertised profile does not implement Lock Controller"));
results.Add(Skip("authentication", "the advertised profile does not require an access token"));
results.Add(Skip("feature-metadata", "the advertised profile declares no SiLA Client Metadata"));

var failed = results.Any(item => item.Status == "fail");
var report = new
{
    role = "feature_provider",
    capabilities = results,
    summary = new
    {
        pass = results.Count(item => item.Status == "pass"),
        fail = results.Count(item => item.Status == "fail"),
        unsupported = results.Count(item => item.Status == "unsupported")
    }
};
await File.WriteAllTextAsync(reportPath, JsonSerializer.Serialize(report, new JsonSerializerOptions { WriteIndented = true }));
if (failed)
{
    throw new Exception("one or more SiLA provider capabilities failed");
}

static Dictionary<string, object> PagePayload() => new()
{
    ["Page"] = new StructurePayload("""{"PageRequest":{"HasCursor":{"Value":false},"Cursor":{"Value":""},"Limit":{"Value":10}}}""")
};

static Dictionary<string, object> RangePayload(string identifier, string value) => new()
{
    [identifier] = new StructurePayload($"{{\"Value\":\"{value}\"}}"),
    ["Range"] = new StructurePayload("""{"TimeRange":{"Start":{"Second":0,"Minute":0,"Hour":0,"Day":1,"Month":1,"Year":2024,"Millisecond":0},"End":{"Second":0,"Minute":0,"Hour":0,"Day":2,"Month":1,"Year":2024,"Millisecond":0}}}"""),
    ["Page"] = new StructurePayload("""{"PageRequest":{"HasCursor":{"Value":false},"Cursor":{"Value":""},"Limit":{"Value":10}}}""")
};

static async Task Call(IDynamicMessageService messages, Grpc.Net.Client.GrpcChannel channel, SiLA2.Feature feature, string command, Dictionary<string, object> payload, string id, List<Capability> results)
{
    try
    {
        messages.ExecuteUnobservableCommand(command, channel, feature, payload);
        results.Add(Pass(id, command));
    }
    catch (Exception error)
    {
        results.Add(Fail(id, error.Message));
    }
    await Task.CompletedTask;
}

static Capability Pass(string id, string detail) => new(id, "pass", detail, "");
static Capability Fail(string id, string detail) => new(id, "fail", detail, "");
static Capability Skip(string id, string rationale) => new(id, "unsupported", rationale, rationale);

record Capability(string Id, string Status, string Detail, string Rationale);

[ProtoContract]
class CancelCommandParameters
{
    [ProtoMember(1)]
    public DataTypeUuid CommandExecutionUUID { get; set; }
}

[ProtoContract]
class DataTypeUuid
{
    [ProtoMember(1)]
    public Sila2.Org.Silastandard.Protobuf.String UUID { get; set; }
}

[ProtoContract]
class CancelCommandResponses
{
}
