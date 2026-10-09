---
id: doc-10
title: A2A-LAB devkit overview
type: overview
audience: public
created_date: "2026-09-30 17:38"
---

# A2A-LAB devkit overview

A2A-LAB devkit routes client calls to lab logs, metrics, tasks, and images. Those values are the [A2A-LAB primitives](<../reference/primitives/doc-17 - A2A-LAB-primitives.md>). Agent2Agent (A2A) and Model Context Protocol (MCP) expose all twelve operations. Standardization in Lab Automation (SiLA) 2 serves logs, metrics, tasks, and named-source images. The crate name is `a2a-lab-dev-kit`.

The standalone [A2A-LAB TCK](https://github.com/A3Analytics/a2a-lab-tck/tree/1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd) checks deterministic lab behavior over A2A and MCP. It is separate from the upstream A2A protocol TCK and does not establish certification.

## Interfaces and providers

The following diagram shows the interfaces and providers around `A2aLabService`. [Interfaces and providers](<../reference/interfaces-and-providers/doc-19 - Interfaces-and-providers.md>) defines those terms.

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

In the preceding diagram, Agent2Agent (A2A), Model Context Protocol (MCP), and Standardization in Lab Automation (SiLA) 2 call `A2aLabService`. AAS, Open Platform Communications Unified Architecture (OPC UA), Robot Operating System 2 (ROS 2), SiLA 2, and Custom fulfill logs, metrics, and tasks. A custom provider can also fulfill images. `A2aLabService` uses those providers. SiLA 2 is both the inbound interface and a remote provider. The inbound server exposes named-source images. The remote provider does not.

## Related pages

Read these pages:

- [Interfaces and providers](<../reference/interfaces-and-providers/doc-19 - Interfaces-and-providers.md>)
- [A2A-LAB primitives](<../reference/primitives/doc-17 - A2A-LAB-primitives.md>)
- [Implementations](<../reference/implementations/doc-18 - Implementations.md>)
- [Run the memory lab](<../guide/memory-lab/doc-12 - Run-the-memory-lab.md>)
- [Serve A2A and MCP](<../guide/a2a-and-mcp/doc-13 - Serve-A2A-and-MCP.md>)
- [Expose ROS 2 actions as tasks](<../guide/ros2-tasks/doc-15 - Expose-ROS-2-actions-as-tasks.md>)
- [Implement a custom provider](<../guide/custom-provider/doc-21 - Implement-a-custom-provider.md>)
- [A2A-LAB TCK](https://github.com/A3Analytics/a2a-lab-tck/tree/1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd)
- [README](../../../README.md)
