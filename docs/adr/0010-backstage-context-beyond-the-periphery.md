---
status: accepted
---

# Backstage context beyond the periphery: system, tags and missing components

ADR-0007 made the catalog push read, of the system's own `Component`, the `resource:` refs in
`spec.dependsOn` and (since 2.6.0) the owner, description, source location and links. Everything
else was ignored on purpose. A review of the graph with its users asked for three more things the
same file already says, and this record amends ADR-0007 for exactly those three.

1. **`spec.system` fills an empty domain.** The Backstage system a Component belongs to is the
   natural cluster of the graph, and Sanshain already draws clusters by the Producer's domain. A
   push sets the domain when none is set. A domain an administrator typed is never overwritten by a
   push: the push is automatic and repeated, the administrator's choice is deliberate, and a build
   must not undo it on every run.
2. **`metadata.tags` are kept.** Shown as chips with the catalog context on the node panel and on
   the system's page, answered by the MCP `describe_service`, and usable as a graph focus
   (`tag:<name>`) that narrows the drawing to the tagged systems and their neighbours. Tags are free
   text per team; Sanshain shows them, it does not interpret them.
3. **`component:` refs without a Sanshain pendant are drawn as missing.** ADR-0007 ignored
   `component:` refs because an edge to a system Sanshain already knows would duplicate the `require`
   edge, and matching Backstage names to Sanshain names is unreliable. Both reasons hold where a
   pendant exists, so such refs stay ignored. A ref whose lower-cased name matches no Producer and
   no Consumer is a different case: it names a dependency the graph cannot see at all. It is drawn
   as a dashed node, named as in Backstage, with a periphery-style edge from the depending system,
   and it travels like the periphery into cuts and diffs. The moment a system of that name provides
   or requires, the dashed node disappears and the `require` edges take over.

Still ignored: `consumesApis`, `providesApis`, lifecycle, every other entity in the file. Edges
between systems that Sanshain knows remain Sanshain's own.
