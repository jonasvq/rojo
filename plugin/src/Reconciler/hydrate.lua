--[[
	Defines the process of "hydration" -- matching up a virtual DOM with
	concrete instances and assigning them IDs.
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)

local invariant = require(script.Parent.Parent.invariant)
local countMatchingProperties = require(script.Parent.countMatchingProperties)

-- When several existing children share a Name and ClassName we disambiguate
-- them by scoring how well each one's properties match the virtual instance.
-- That scoring is far more expensive than a Name/ClassName check, so we only do
-- it for reasonably-sized groups. Larger groups (e.g. a folder with thousands of
-- identically-named parts) fall back to the original order-based matching, which
-- bounds the added work to roughly MAX_CANDIDATES_TO_SCORE^2 property reads per
-- group regardless of how large the group is.
local MAX_CANDIDATES_TO_SCORE = 32

local function hydrateInner(
	stats,
	instanceMap,
	virtualInstances,
	rootId,
	rootInstance,
	rememberedInstances,
	removedByParent
)
	local virtualInstance = virtualInstances[rootId]

	if virtualInstance == nil then
		invariant("Cannot hydrate an instance not present in virtualInstances\nID: {}", rootId)
	end

	instanceMap:insert(rootId, rootInstance)
	stats.hydrated += 1

	local existingChildren = rootInstance:GetChildren()

	-- Group existing children by Name then ClassName so each virtual child can
	-- find its candidate matches without scanning every sibling. This is what
	-- keeps hydration fast for parents with thousands of children. Nesting the
	-- two tables (rather than a combined key) keeps the Name and ClassName checks
	-- exact, with no way for one to bleed into the other.
	local buckets = {}
	for _, childInstance in existingChildren do
		-- We guard accessing Name and ClassName in order to avoid tripping over
		-- children of DataModel that Rojo won't have permissions to access at all.
		local accessSuccess, name, className = pcall(function()
			return childInstance.Name, childInstance.ClassName
		end)
		if not accessSuccess then
			continue
		end

		local bucketsByClassName = buckets[name]
		if bucketsByClassName == nil then
			bucketsByClassName = {}
			buckets[name] = bucketsByClassName
		end

		local bucket = bucketsByClassName[className]
		if bucket == nil then
			bucket = { cursor = 1, instances = {} }
			bucketsByClassName[className] = bucket
		end

		table.insert(bucket.instances, childInstance)
	end

	-- Tracks which existing children have already been paired, so one instance
	-- isn't matched to two different virtual instances.
	local visited = {}
	if removedByParent and removedByParent[rootInstance] then
		for id, instance in removedByParent[rootInstance] do
			-- Keep deletions identifiable even when this parent preserves unknown
			-- children. Reserve them so a newly created ID cannot reuse the object.
			instanceMap:insert(id, instance)
			visited[instance] = true
		end
	end
	local rememberedChildren = {}
	if rememberedInstances then
		-- Reserve known identities before heuristic matching, otherwise an
		-- earlier sibling could consume a later sibling's remembered instance.
		for _, childId in virtualInstance.Children do
			local candidate = rememberedInstances[childId]
			if candidate and candidate.Parent == rootInstance and not visited[candidate] then
				visited[candidate] = true
				rememberedChildren[childId] = candidate
			end
		end
	end

	for _, childId in ipairs(virtualInstance.Children) do
		local virtualChild = virtualInstances[childId]
		local remembered = rememberedChildren[childId]
		if remembered then
			hydrateInner(
				stats,
				instanceMap,
				virtualInstances,
				childId,
				remembered,
				rememberedInstances,
				removedByParent
			)
			continue
		end

		local bucketsByClassName = buckets[virtualChild.Name]
		local bucket = bucketsByClassName and bucketsByClassName[virtualChild.ClassName]
		if bucket == nil then
			-- No existing instance matches; diff will mark this id for creation.
			Log.trace(
				"hydrate: no existing instance matches {} ({}) for id {}",
				virtualChild.Name,
				virtualChild.ClassName,
				childId
			)
			continue
		end

		local instances = bucket.instances

		-- Advance past any leading children that have already been paired. The
		-- cursor makes order-based matching amortized O(1) per child even for
		-- very large groups, rather than rescanning the visited prefix.
		while bucket.cursor <= #instances and visited[instances[bucket.cursor]] do
			bucket.cursor += 1
		end
		if bucket.cursor > #instances then
			-- Every matching instance has already been paired with an earlier id.
			Log.trace(
				"hydrate: no unpaired instance left for {} ({}) for id {}",
				virtualChild.Name,
				virtualChild.ClassName,
				childId
			)
			continue
		end

		-- The cursor points at the earliest unvisited child, so the slots from
		-- here to the end bound how many candidates remain. Visited children
		-- after the cursor (gaps) only appear once a group is small enough to be
		-- scored -- the order-based path below always takes the earliest, which
		-- keeps the visited region a contiguous prefix. So whenever this count
		-- exceeds the cap it is exact, and we can pick the earliest match without
		-- collecting anything.
		local remaining = #instances - bucket.cursor + 1

		local match
		if remaining > MAX_CANDIDATES_TO_SCORE then
			-- Too many to score affordably; take the earliest in child order,
			-- reproducing the original Name + ClassName behavior.
			match = instances[bucket.cursor]
			Log.trace(
				"hydrate: {} candidates named {} ({}) exceeds the scoring cap of {}; matching id {} by child order",
				remaining,
				virtualChild.Name,
				virtualChild.ClassName,
				MAX_CANDIDATES_TO_SCORE,
				childId
			)
		else
			-- Collect the (at most `remaining`) unvisited candidates.
			local candidates = {}
			for index = bucket.cursor, #instances do
				local childInstance = instances[index]
				if not visited[childInstance] then
					table.insert(candidates, childInstance)
				end
			end

			if #candidates == 1 then
				-- Only one candidate, so there's nothing to disambiguate.
				match = candidates[1]
			else
				-- Break the tie by choosing the candidate whose properties best
				-- match the virtual instance, falling back to the earliest in
				-- child order when scores are equal.
				local bestScore = -1
				for _, childInstance in candidates do
					local score = countMatchingProperties(childInstance, virtualChild, instanceMap)
					if score > bestScore then
						bestScore = score
						match = childInstance
					end
				end

				stats.ambiguousGroups += 1
				stats.candidatesScored += #candidates
				Log.trace(
					"hydrate: disambiguated {} candidates named {} ({}) for id {} by property match (best score {})",
					#candidates,
					virtualChild.Name,
					virtualChild.ClassName,
					childId,
					bestScore
				)
			end
		end

		visited[match] = true
		hydrateInner(stats, instanceMap, virtualInstances, childId, match, rememberedInstances, removedByParent)
	end
end

local function hydrate(instanceMap, virtualInstances, rootId, rootInstance, rememberedInstances)
	-- Tallies of the work hydration did, surfaced in a single debug log below so
	-- the cost of property-based disambiguation is visible without per-node spam.
	local stats = {
		hydrated = 0,
		ambiguousGroups = 0,
		candidatesScored = 0,
	}

	local removedByParent
	if rememberedInstances then
		removedByParent = {}
		for id, instance in rememberedInstances do
			local parent = instance.Parent
			if virtualInstances[id] == nil and parent then
				removedByParent[parent] = removedByParent[parent] or {}
				removedByParent[parent][id] = instance
			end
		end
	end
	hydrateInner(stats, instanceMap, virtualInstances, rootId, rootInstance, rememberedInstances, removedByParent)

	Log.debug(
		"Hydrated {} instances ({} ambiguous name+class groups, {} candidates scored)",
		stats.hydrated,
		stats.ambiguousGroups,
		stats.candidatesScored
	)
end

return hydrate
