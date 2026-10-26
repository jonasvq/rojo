//! Live guards for the independent class-authority configuration (#1301).

use anyhow::ensure;

use crate::write_receipts::bounded;

pub(crate) fn check_live_authority(
    snapshot: Option<&crate::InstanceSnapshot>,
) -> anyhow::Result<()> {
    let Some(root) = snapshot else {
        return Ok(());
    };
    if root.metadata.context.sourcemap_only || root.metadata.context.offline_session {
        return Ok(());
    }
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if let Some(authority) = &node.metadata.effective_sync_authority {
            if !authority.policies().is_empty() {
                authority.resolve(node.class_name.as_str())?;
            }
            ensure!(authority.policies().iter().all(|policy| policy.properties != crate::PropertyAuthority::TwoWay && policy.structure == crate::StructureAuthority::Filesystem),
                "syncAuthority twoWay live persistence is unavailable after the general write feature was withdrawn; use filesystem structure and filesystem/studio properties with reviewed client authority (instance {})", bounded(&node.name));
        }
        pending.extend(&node.children);
    }
    Ok(())
}
