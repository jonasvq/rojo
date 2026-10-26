return function()
	local ApiContext = require(script.Parent.ApiContext)
	local Config = require(script.Parent.Config)
	local Settings = require(script.Parent.Settings)
	local Http = require(script.Parent.Parent.Packages.Http)
	local Promise = require(script.Parent.Parent.Packages.Promise)

	local function responseFor(value)
		return Http.Response.fromRobloxResponse({ StatusCode = 200, Body = Http.msgpackEncode(value), Headers = {} })
	end

	local function fixture(callback)
		local previousGet = Http.get
		local previousChecking = Settings:get("typecheckingEnabled")
		local api = ApiContext.new("http://127.0.0.1:34872")
		local ok, err = pcall(callback, api)
		api:disconnect()
		Http.get = previousGet
		Settings:set("typecheckingEnabled", previousChecking)
		assert(ok, tostring(err))
	end

	it("rejects scalar and malformed handshake bodies before indexing fields with typechecking disabled", function()
		fixture(function(api)
			Settings:set("typecheckingEnabled", false)
			for _, value in
				{
					1,
					false,
					"not a server",
					{},
					{ protocolVersion = Config.protocolVersion },
					{ protocolVersion = "5" },
				}
			do
				Http.get = function()
					return Promise.resolve(responseFor(value))
				end
				local ok, reason = api:connect():await()
				expect(ok).to.equal(false)
				local diagnostic = tostring(reason)
				assert(diagnostic:find("Invalid Rojo server handshake", 1, true), diagnostic)
				assert(diagnostic:find("http://127.0.0.1:34872", 1, true), diagnostic)
				expect(api.__sessionId).to.equal(nil)
			end
		end)
	end)

	it("retains valid handshake capabilities and useful incompatible-protocol diagnostics", function()
		fixture(function(api)
			for _, checking in { true, false } do
				Settings:set("typecheckingEnabled", checking)
				local value = {
					sessionId = "valid-session",
					rootInstanceId = "ROOT",
					serverVersion = "7.7.1",
					protocolVersion = Config.protocolVersion,
					capabilities = { "classAuthorityV1", "propertyRemovalsV1" },
				}
				Http.get = function()
					return Promise.resolve(responseFor(value))
				end
				local ok, body = api:connect():await()
				expect(ok).to.equal(true)
				expect(body.sessionId).to.equal("valid-session")
				value.protocolVersion = Config.protocolVersion + 1
				local accepted, reason = api:connect():await()
				expect(accepted).to.equal(false)
				assert(tostring(reason):find("different protocol version", 1, true), tostring(reason))
			end
		end)
	end)

	it("rejects incomplete MessagePack and nil handshake data without adopting session state", function()
		fixture(function(api)
			for _, bytes in { string.char(0xd9), string.char(0xc0) } do
				Http.get = function()
					return Promise.resolve(
						Http.Response.fromRobloxResponse({ StatusCode = 200, Body = bytes, Headers = {} })
					)
				end
				local ok, reason = api:connect():await()
				expect(ok).to.equal(false)
				assert(tostring(reason):find("Invalid Rojo server handshake", 1, true), tostring(reason))
				expect(api.__sessionId).to.equal(nil)
			end
		end)
	end)
end
