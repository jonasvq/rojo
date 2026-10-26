local HttpService = game:GetService("HttpService")

local OpenLaunch = {}
OpenLaunch.markerName = "__RojoOpenLaunch_v1"
local fields = {
	version = true,
	host = true,
	port = true,
	sessionId = true,
	projectName = true,
	createdAt = true,
	expiresAt = true,
}

function OpenLaunch.validate(details, now)
	if type(details) ~= "table" then
		return nil, "Open launch details must be an object"
	end
	for name in details do
		if not fields[name] then
			return nil, "Unknown open launch field"
		end
	end
	if details.version ~= 1 or details.host ~= "127.0.0.1" then
		return nil, "Unsupported open launch version or non-loopback host"
	end
	if type(details.port) ~= "number" or details.port % 1 ~= 0 or details.port < 1 or details.port > 65535 then
		return nil, "Invalid open launch port"
	end
	if
		type(details.sessionId) ~= "string"
		or not details.sessionId:match("^%x%x%x%x%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$")
	then
		return nil, "Invalid open launch session"
	end
	if type(details.projectName) ~= "string" or #details.projectName > 256 or details.projectName:find("%c") then
		return nil, "Invalid open launch project name"
	end
	if
		type(details.createdAt) ~= "number"
		or type(details.expiresAt) ~= "number"
		or details.createdAt % 1 ~= 0
		or details.expiresAt % 1 ~= 0
		or details.createdAt < 0
		or details.expiresAt - details.createdAt ~= 600
		or details.createdAt > now + 30
		or now > details.expiresAt
	then
		return nil, "Expired or invalid open launch time"
	end
	return table.freeze(table.clone(details))
end

function OpenLaunch.consume(storage, now)
	storage = storage or game:GetService("ReplicatedStorage")
	now = now or DateTime.now().UnixTimestamp
	local marker
	for _, child in storage:GetChildren() do
		if child.Name == OpenLaunch.markerName then
			if marker then
				return nil, "Ambiguous open launch markers"
			end
			marker = child
		end
	end
	if not marker then
		return nil
	end
	local children = marker:GetChildren()
	if
		not marker:IsA("Folder")
		or #children ~= 1
		or children[1].Name ~= "Details"
		or not children[1]:IsA("StringValue")
		or #children[1].Value > 4096
	then
		return nil, "Malformed open launch marker; left unrecognized content intact"
	end
	local ok, details = pcall(HttpService.JSONDecode, HttpService, children[1].Value)
	if not ok or type(details) ~= "table" or details.version ~= 1 then
		return nil, "Malformed open launch details; left unrecognized content intact"
	end
	-- Only the recognized descriptor shape belongs to this workflow. Never
	-- destroy unrelated content just because it has the reserved name.
	for name in details do
		if not fields[name] then
			return nil, "Unknown open launch fields; left unrecognized content intact"
		end
	end
	marker.Archivable = false
	marker:Destroy()
	return OpenLaunch.validate(details, now)
end

return OpenLaunch
