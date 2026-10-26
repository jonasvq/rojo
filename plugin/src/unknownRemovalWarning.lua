-- Initial diffs identify unknown removals by Instance; managed removals use IDs.
return function(patch)
	local candidates = {}
	for _, removed in patch.removed do
		if typeof(removed) == "Instance" then
			candidates[removed] = true
		end
	end
	local roots, descendants = 0, 0
	local paths = {}
	for instance in candidates do
		local parent = instance.Parent
		local covered = false
		while parent do
			if candidates[parent] then
				covered = true
				break
			end
			parent = parent.Parent
		end
		if not covered then
			roots += 1
			descendants += #instance:GetDescendants()
			if #paths < 10 then
				local path = instance:GetFullName()
				if #path > 200 then
					path = string.sub(path, 1, 197) .. "..."
				end
				table.insert(paths, path)
			end
		end
	end
	if roots == 0 then
		return nil
	end
	table.sort(paths)
	return string.format(
		"Initial sync will remove %d unknown Studio roots and %d descendants. Paths (up to 10): %s. "
			.. "To preserve unknown instances, set $ignoreUnknownInstances on their parent, or $ignoreUnknownDescendants on a project subtree.",
		roots,
		descendants,
		table.concat(paths, "; ")
	)
end
