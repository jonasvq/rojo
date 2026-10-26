return function(info, placeId)
	if info.expectedPlaceIds ~= nil and not table.find(info.expectedPlaceIds, placeId) then
		local ids = {}
		for _, id in info.expectedPlaceIds do
			table.insert(ids, "- " .. tostring(id))
		end
		return (
			"Found a Rojo server, but its project is set to only be used with a specific list of places."
			.. "\nYour place ID is %u, but needs to be one of these:\n%s"
			.. "\n\nTo change this list, edit 'servePlaceIds' in your .project.json file."
		):format(placeId, table.concat(ids, "\n"))
	end
	if info.unexpectedPlaceIds ~= nil and table.find(info.unexpectedPlaceIds, placeId) then
		local ids = {}
		for _, id in info.unexpectedPlaceIds do
			table.insert(ids, "- " .. tostring(id))
		end
		return (
			"Found a Rojo server, but its project is set to not be used with a specific list of places."
			.. "\nYour place ID is %u, but needs to not be one of these:\n%s"
			.. "\n\nTo change this list, edit 'blockedPlaceIds' in your .project.json file."
		):format(placeId, table.concat(ids, "\n"))
	end
	return nil
end
