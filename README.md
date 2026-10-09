# A3 Analytics A2A-LAB devkit

<p align="center">
  <a href="https://a3analytics.ai/"><img src="https://a3analytics.ai/logo/logo.svg" alt="A3 Analytics logo" width="360"></a>
</p>

[![A2A 1.0](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/actions/workflows/a2a-tck.yml/badge.svg)](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/actions/workflows/a2a-tck.yml)

The A3 Analytics A2A-LAB devkit is a Rust crate for lab logs, metrics, tasks, and images. Agent2Agent (A2A) and Model Context Protocol (MCP) expose all twelve operations. SiLA 2 serves logs, metrics, tasks, and named-source images. AAS, OPC UA, and ROS 2 stay on logs, metrics, and tasks.

## Features

- Supports lab logs, metrics, tasks, and images over Agent2Agent (A2A) and Model Context Protocol (MCP).
- Serves logs, metrics, tasks, and named-source images to Standardization in Lab Automation (SiLA) 2 clients. Image bytes use SiLA binary download.
- Connects to a remote SiLA 2 server and exposes configured members as lab tasks, logs, and metrics.
- Reads equipment from an AAS catalog.
- Reads Open Platform Communications Unified Architecture (OPC UA) equipment history.
- Supplies live OPC UA readings through `LiveSource`. [Upcoming]
- Exposes Robot Operating System 2 (ROS 2) actions as lab tasks.

## Interfaces and providers

The following diagram shows the interfaces and providers around `A2aLabService`:

```mermaid
flowchart LR
  accTitle: A2A-LAB devkit
  accDescr: A2A, MCP, and SiLA 2 call A2aLabService. A2aLabService uses AAS, OPC UA, ROS 2, SiLA 2, and Custom.
  subgraph interfaces [Interfaces]
    a2a["A2A"]
    mcp["MCP"]
    sila["SiLA 2"]
  end
  lab["A2aLabService"]
  subgraph providers [Providers]
    aas["AAS"]
    opc["OPC UA"]
    ros["ROS 2"]
    silaProvider["SiLA 2"]
    custom["Custom"]
  end
  interfaces --> lab
  lab --> providers
```

A2A, MCP, and SiLA 2 call `A2aLabService`. `A2aLabService` uses AAS, OPC UA, ROS 2, SiLA 2, and Custom. SiLA 2 is both the inbound interface and a remote provider. The inbound server also serves named-source images through binary download.

## Run the logs, metrics, and tasks example

Run these commands from the repository root.

1. Install the tools:

   ```bash
   mise install
   ```

2. Run the [logs, metrics, and tasks example](examples/logs_metrics_tasks.rs):

   ```bash
   mise exec -- cargo run --example logs_metrics_tasks
   ```

## Check A2A-LAB compliance

A2A-LAB compliance tests deterministic lab-operation behavior and A2A/MCP parity. The authoritative [contract](https://github.com/A3Analytics/a2a-lab-tck/blob/1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd/backlog/docs/reference/compliance/doc-1%20-%20A2A-LAB-TCK-profile.md), [runner](https://github.com/A3Analytics/a2a-lab-tck/blob/1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd/src/compliance_runner.rs), and [GitHub Action](https://github.com/A3Analytics/a2a-lab-tck/blob/1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd/action.yml) are maintained in the standalone [A2A-LAB TCK](https://github.com/A3Analytics/a2a-lab-tck). Pin consumers to the full published commit SHA `1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd`.

The devkit `v0.1.0` release at `9d5327868d96b3e800fd89f6debf434bcc12709d` is the last release with the embedded TCK. Its Action contract remains immutable. The current repository-root Action is a deprecated compatibility bridge to the standalone revision.

Use the standalone [A2A-LAB TCK documentation](https://github.com/A3Analytics/a2a-lab-tck/blob/1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd/README.md) to create fixtures, pin its Action, retain the JSON report, and display the consumer repository's workflow badge. That badge reports only the consumer-owned workflow result; it is not certification or a devkit badge.

## Wiki

Read the [A2A-LAB devkit overview](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/Home).

## Open the crate documentation

Generate the application programming interface (API) documentation with this command:

```bash
mise exec -- cargo doc --no-deps --open
```

## Check the crate

1. Build the crate:

   ```bash
   mise run build
   ```

2. Test the crate:

   ```bash
   mise run test
   ```

3. Run the quality checks:

   ```bash
   mise run quality
   ```

`mise run quality` checks formatting, the SiLA matrix, the Wiki, complexity, duplication, compilation, Clippy, tests, the A2A TCK, and the SiLA provider checks.

The Wiki action publishes public Backlog docs.
