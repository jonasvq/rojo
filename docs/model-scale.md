# Model scale is serialized state, not an import multiplier

Keep the scale factor and the model's saved geometry together. Roblox exports
already-scaled Size/CFrame values **and** its stored scale factor. Rojo preserves
both. Applying `ScaleTo` again after importing those children would scale a valid
export twice.

For example, a native model whose original pole length is6 has pole length12
after `ScaleTo(2)`. The saved model contains Scale2 and length12. Rojo retains12;
the next native `ScaleTo(3)` produces18, matching Studio. A file containing
Scale2 and length6 describes different saved geometry, not the same model with
an unapplied multiplier. Its implied unit pole length is3.

For repeatable editing:

1. Scale the assembled model in Studio.
2. Save/export the complete resulting model, including geometry and references.
3. Review the file diff and use explicit syncback when importing that saved
   state into a Rojo project. Rebuild/reconnect to verify the saved state.

If editing JSON directly, update the corresponding geometry as well as the
factor. Changing only `Properties.Scale` is not a complete saved-model edit;
it can produce different intermediate live geometry from a fresh import of the
unchanged Size/CFrame fields. This fork adds no new JSON transform mode.

## Regression coverage

The native plugin test checks exported geometry, complete parent/child property
updates at scales3/1/2, instance identity, PrimaryPart and fresh reification:
plugin/src/Reconciler/modelScale.spec.lua.

tests/model_scale_roundtrip.rs checks JSON/XML/binary sources through both
model output formats, retaining the stored factor, literal geometry and
PrimaryPart reference.
