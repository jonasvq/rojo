local Util = {}

function Util.copy(value, seen)
	if type(value) ~= "table" then
		return value
	end
	seen = seen or {}
	assert(not seen[value], "Cyclic API value")
	seen[value] = true
	local result = {}
	for key, item in value do
		result[Util.copy(key, seen)] = Util.copy(item, seen)
	end
	seen[value] = nil
	return result
end

function Util.hostPort(host, port)
	assert(type(host) == "string" and #host > 0 and #host <= 253, "Expected a host")
	local scheme, authority = host:match("^(https?)://(.+)$")
	authority = authority or host
	assert(not authority:find("[%s/%?#@%c]"), "Host must not contain a path, credentials or query")
	assert(authority:match("^[%w%.%-]+$") or authority:match("^%[[%x:%.]+%]$"), "Invalid host")
	port = tonumber(port)
	assert(port and port % 1 == 0 and port >= 1 and port <= 65535, "Port must be an integer from 1 to 65535")
	return (if scheme then scheme .. "://" else "") .. authority:lower(), tostring(port)
end

function Util.baseUrl(url)
	assert(type(url) == "string" and #url <= 2048, "Expected HTTP(S) URL")
	local scheme, authority, path = url:match("^(https?)://([^/]+)(.*)$")
	assert(scheme and not url:find("[%s%?#@%c]"), "Expected HTTP(S) URL without credentials, fragment or query")
	local host, port = authority:match("^(%b[]):(%d+)$")
	if not host then
		host, port = authority:match("^([^:]+):(%d+)$")
	end
	if not host then
		host = authority
	end
	local checkedHost, checkedPort =
		Util.hostPort(scheme .. "://" .. host, port or (if scheme == "https" then "443" else "80"))
	return checkedHost .. ":" .. checkedPort .. path:gsub("/+$", "")
end

function Util.endpoint(host, port)
	return (if host:match("^https?://") then host else "http://" .. host) .. ":" .. port
end

return Util
