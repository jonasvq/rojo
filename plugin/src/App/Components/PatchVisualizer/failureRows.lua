-- Keep ancestors for path context without exposing unrelated successful edits.
return function(tree, failuresOnly)
	if not tree then
		return {}, 0
	end
	local all, included, failures = {}, {}, 0
	tree:forEach(function(node, depth)
		table.insert(all, { node = node, depth = depth })
		if node.isWarning then
			failures += 1
			local current = node
			while current and current.id and not included[current.id] do
				included[current.id] = true
				current = if current.parentId ~= "ROOT" then tree:getNode(current.parentId) else nil
			end
		end
	end)
	local rows, lastChild = {}, {}
	for _, row in all do
		local node = row.node
		if failuresOnly and not included[node.id] then
			continue
		end
		if failuresOnly then
			node = table.clone(node)
			if node.isWarning then
				local changes = {}
				if node.changeList then
					table.insert(changes, node.changeList[1])
					for index = 2, #node.changeList do
						local change = node.changeList[index]
						if change[4] and change[4].isWarning then
							table.insert(changes, change)
						end
					end
				end
				node.changeList = if #changes > 1 then changes else nil
				node.changeInfo = if #changes > 1 then { failed = #changes - 1 } else nil
			else
				node.changeList, node.changeInfo, node.patchType = nil, nil, nil
			end
		end
		table.insert(rows, { node = node, depth = row.depth })
		lastChild[node.parentId or "ROOT"] = node.id
	end
	for index, row in rows do
		row.hasChildren = rows[index + 1] ~= nil and rows[index + 1].depth > row.depth
		row.isFinalChild = lastChild[row.node.parentId or "ROOT"] == row.node.id
	end
	return rows, failures
end
