# Class-based property authority

Use `syncAuthority` to keep scripts in files while preserving selected classes'
property values in Studio:

```json
{
  "syncAuthority": [
    { "className": "LuaSourceContainer", "properties": "filesystem" },
    { "className": "BasePart", "properties": "studio" }
  ]
}
```

Place this field beside the project's `tree`. Exact classes take precedence;
otherwise the nearest configured superclass wins. Classes must exist in the
reflection database and may appear only once. A policy's structure defaults to
filesystem ownership.

Nested omission inherits the enclosing policy list. An explicit list replaces
it; an empty list restores legacy behavior. Each matched entry is a complete
policy rather than a partial merge.

## Live synchronization

Filesystem-owned properties follow the file snapshot. Studio-owned properties
survive initial connection and live file changes through the negotiated
`classAuthorityV1` capability. New instances receive their initial file values;
subsequent updates respect authority. Managed structure still follows files.

Policy changes, destructive replacements and ambiguous recovery require review
before discarding Studio values. A filesystem-owned Model.Scale is rejected when
its native operation would indirectly change Studio-owned descendant geometry.
An incompatible plugin is refused before unsafe sync.

Explicit matching policies also revoke legacy property writes and Source-only
receipt writes for those instances. General live `twoWay` properties/structure
are guarded; parsing a declaration does not grant a write capability.

## Explicit syncback

Save the edited place, then preview its import with `syncback --dry-run --list`.
Filesystem authority retains the complete accepted property set, including
absent/default values, attributes, tags and Source. Studio authority selects the
imported fields. Structure and references must remain unambiguous and safe.

Native parent source bytes can remain unchanged while Studio-owned descendants
export. Existing source-format and layout restrictions still apply. Review the
planned paths, apply the exact saved input, inspect the diff and rebuild.

Class exclusions use the separate `syncbackRules.ignoreClasses` setting. They
preserve whole existing subtrees; property authority operates per instance.

See the [cookbook](rojo-cookbook.md) and
[fully managed workflow](fully-managed-workflow.md) for complete commands.
