---
id: doc-16
title: Wiki publishing
type: technical
audience: technical
created_date: "2026-10-06 20:12"
---

# Wiki publishing

## Purpose

Public Backlog docs are the Wiki source. The Wiki GitHub Action publishes those pages. Edit the docs in this repository.

## Sources

A public doc sets `audience: public` and lives under `overview`, `guide`, or `reference`. `.mise/wiki-map.toml` maps each directory to one Wiki file. Map order is the sidebar order: Start, Guides, then Reference.

Each public page has one heading 1, and that heading matches `title`. Mermaid is optional. A diagram starts with `accTitle` and `accDescr`. The following prose restates every node and relationship. The publisher keeps the Mermaid fence so GitHub renders it.

Technical docs stay in the repository. Public pages do not link to them.

## Checks

`mise run wiki-check` writes `target/wiki-stage` and checks headings, fences, links, diagram text, and README link order. A pull request runs that check without a Wiki token. The check covers docs, `README.md`, the map, and the Wiki scripts.

## Publication

The `main` workflow needs the `WIKI_TOKEN` repository secret. Use a fine-grained token with Wikis read and write. `GITHUB_TOKEN` cannot write a Wiki.

Create one page in the GitHub Wiki tab before the first publish. That creates `.wiki.git`. The workflow then publishes the stage and compares it with the remote. The publish does not force-push.

With `WIKI_TOKEN` set, `mise run wiki-drift` compares the local stage with the remote Wiki. It does not push.
