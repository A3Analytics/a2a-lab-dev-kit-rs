---
id: doc-14
title: Run a scripted industrial lab
type: guide
audience: public
created_date: '2026-10-06 20:11'
---

# Run a scripted industrial lab

## Query a scripted metric

Run the [scripted industrial example](../../../../examples/industrial_scripted.rs) to query one metric. The Open Platform Communications Unified Architecture (OPC UA) endpoint on the binding is catalog data. Samples come from `ScriptedLive`.

`IndustrialLabBuilder` pairs `MemoryCatalog` with `ScriptedLive`. The query reads metric `temperature`.

1. Run the example:

```bash
mise exec -- cargo run --example industrial_scripted
```

The example stdout is:

```text
scripted temperature sample 21.5
```

## Related

- [Lab dev kit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [Asset Administration Shell](<../../reference/aas/doc-6 - Asset-Administration-Shell.md>)
- [OPC UA](<../../reference/opc-ua/doc-7 - OPC-UA.md>)
