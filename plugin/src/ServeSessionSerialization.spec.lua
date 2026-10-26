return function()
	local Packages = script.Parent.Parent.Packages
	local Promise = require(Packages.Promise)
	local ServeSession = require(script.Parent.ServeSession)
	local PatchSet = require(script.Parent.PatchSet)

	it("isolates one broken serialized asset instead of blocking a healthy replacement", function()
		local models = Instance.new("Folder")
		local oldGood = Instance.new("Part")
		oldGood.Name = "Good"
		oldGood.Parent = models
		local oldBad = Instance.new("Part")
		oldBad.Name = "Bad"
		oldBad.Parent = models
		local root = Instance.new("Folder")
		local object = Instance.new("ObjectValue")
		object.Name = "GOOD"
		object.Parent = root
		local replacement = Instance.new("Part")
		replacement.Name = "Good"
		replacement.Anchored = true
		replacement.Parent = object
		object.Value = replacement
		local contents = game:GetService("SerializationService"):SerializeInstancesAsync({ root })
		root:Destroy()
		local api = {}
		function api:serialize(ids)
			return Promise.resolve({
				modelContents = if table.find(ids, "BAD") then buffer.fromstring("broken model") else contents,
			})
		end
		function api:refPatch()
			return Promise.resolve({ patch = PatchSet.newEmpty() })
		end
		function api:read()
			return Promise.resolve({ instances = { GOOD = { Properties = {} } } })
		end
		function api:disconnect() end
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		session.__instanceMap:insert("GOOD", oldGood)
		session.__instanceMap:insert("BAD", oldBad)
		local ok, err = pcall(function()
			local success = session:__replaceInstances({ "GOOD", "BAD" })
			expect(success).to.equal(true)
			expect(session.__instanceMap.fromIds.GOOD == oldGood).to.equal(false)
			expect(session.__instanceMap.fromIds.GOOD.Anchored).to.equal(true)
			expect(session.__instanceMap.fromIds.BAD).to.equal(oldBad)
			expect(oldBad.Parent).to.equal(models)
		end)
		session:stop()
		oldGood:Destroy()
		models:Destroy()
		assert(ok, tostring(err))
	end)

	local Selection = game:GetService("Selection")
	local function contents(id, class, malformed)
		local root = Instance.new("Folder")
		local object = Instance.new(if malformed then "Folder" else "ObjectValue")
		object.Name = id
		object.Parent = root
		local value = Instance.new(class or "Part")
		value.Name = id
		value.Parent = object
		if not malformed then
			object.Value = value
		end
		local result = game:GetService("SerializationService"):SerializeInstancesAsync({ root })
		root:Destroy()
		return result
	end

	local function fixture(callback)
		local parent = Instance.new("Folder")
		parent.Parent = workspace
		local good, bad = Instance.new("ObjectValue"), Instance.new("Part")
		good.Name, bad.Name = "Good", "Bad"
		good.Parent, bad.Parent = parent, parent
		local api = { __serializedModels = true }
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		session.__instanceMap:insert("GOOD", good)
		session.__instanceMap:insert("BAD", bad)
		local previousSelection = Selection:Get()
		function api:disconnect() end
		local ok, err = pcall(callback, session, api, good, bad, parent)
		session:stop()
		Selection:Set(previousSelection)
		good:Destroy()
		bad:Destroy()
		parent:Destroy()
		assert(ok, tostring(err))
	end

	it("replaces healthy modern models while retaining failed IDs, children and selection", function()
		fixture(function(session, api, good, bad, parent)
			local child = Instance.new("Folder")
			child.Parent = good
			Selection:Set({ bad, good })
			function api:serialize()
				return Promise.resolve({
					models = {
						{ id = "GOOD", modelContents = contents("GOOD", "ObjectValue") },
						{ id = "BAD", error = "cannot encode asset" },
					},
					refPatch = PatchSet.newEmpty(),
				})
			end
			local ok, _, replaced = session:__replaceInstances({ "GOOD", "BAD" })
			expect(ok).to.equal(true)
			expect(replaced.GOOD).to.equal(true)
			expect(replaced.BAD).to.equal(nil)
			local replacement = session.__instanceMap.fromIds.GOOD
			expect(replacement == good).to.equal(false)
			expect(replacement.Parent).to.equal(parent)
			expect(child.Parent).to.equal(replacement)
			expect(Selection:Get()[1]).to.equal(bad)
			expect(Selection:Get()[2]).to.equal(replacement)
		end)
	end)

	it("rejects malformed, duplicate and missing entries without marking failed roots successful", function()
		for _, mode in { "broken", "wrapper", "duplicate", "missing", "unexpected", "envelope", "allFailed" } do
			fixture(function(session, api, good, bad)
				function api:serialize()
					local entry = { id = "GOOD", modelContents = contents("GOOD", "ObjectValue", mode == "wrapper") }
					if mode == "broken" then
						entry.modelContents = buffer.fromstring("bad")
					end
					local models = { entry, { id = "BAD", error = "bad" } }
					if mode == "duplicate" then
						table.insert(models, entry)
					end
					if mode == "missing" then
						table.remove(models, 1)
					end
					if mode == "unexpected" then
						table.insert(models, { id = "OTHER" })
					end
					if mode == "allFailed" then
						models[1] = { id = "GOOD", error = "bad" }
					end
					return Promise.resolve({
						models = models,
						refPatch = if mode == "envelope" then {} else PatchSet.newEmpty(),
					})
				end
				expect(session:__replaceInstances({ "GOOD", "BAD" })).to.equal(false)
				expect(session.__instanceMap.fromIds.GOOD).to.equal(good)
				expect(session.__instanceMap.fromIds.BAD).to.equal(bad)
			end)
		end
	end)

	it("restores inbound, outbound, cross-model and explicit nil references", function()
		fixture(function(session, api, good, _, parent)
			local other = Instance.new("ObjectValue")
			other.Parent = parent
			local inbound = Instance.new("ObjectValue")
			inbound.Parent = parent
			inbound.Value = good
			session.__instanceMap:insert("OTHER", other)
			session.__instanceMap:insert("IN", inbound)
			function api:serialize()
				return Promise.resolve({
					models = {
						{ id = "GOOD", modelContents = contents("GOOD", "ObjectValue") },
						{ id = "OTHER", modelContents = contents("OTHER", "ObjectValue") },
					},
					refPatch = {
						added = {},
						removed = {},
						updated = {
							{ id = "GOOD", changedProperties = { Value = { Ref = "OTHER" } } },
							{
								id = "OTHER",
								changedProperties = { Value = { Ref = "00000000000000000000000000000000" } },
							},
							{ id = "IN", changedProperties = { Value = { Ref = "GOOD" } } },
						},
					},
				})
			end
			local ok, unapplied = session:__replaceInstances({ "GOOD", "OTHER" })
			expect(ok).to.equal(true)
			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(session.__instanceMap.fromIds.GOOD.Value).to.equal(session.__instanceMap.fromIds.OTHER)
			expect(session.__instanceMap.fromIds.OTHER.Value).to.equal(nil)
			expect(inbound.Value).to.equal(session.__instanceMap.fromIds.GOOD)
			other:Destroy()
		end)
	end)

	it("fetches legacy outbound references and aborts when reference fetching fails", function()
		for _, fail in { false, true } do
			fixture(function(session, api, good, bad)
				api.__serializedModels = false
				function api:serialize(ids)
					expect(#ids).to.equal(1)
					return Promise.resolve({ modelContents = contents(ids[1], "ObjectValue") })
				end
				function api:refPatch()
					return Promise.resolve({ patch = PatchSet.newEmpty() })
				end
				function api:read()
					if fail then
						return Promise.reject("read failed")
					end
					return Promise.resolve({ instances = { GOOD = { Properties = { Value = { Ref = "BAD" } } } } })
				end
				local ok = session:__replaceInstances({ "GOOD" })
				expect(ok).to.equal(not fail)
				if fail then
					expect(session.__instanceMap.fromIds.GOOD).to.equal(good)
				else
					expect(session.__instanceMap.fromIds.GOOD.Value).to.equal(bad)
				end
			end)
		end
	end)

	it("aborts late swaps after stopping during a serialization request", function()
		fixture(function(session, api, good)
			function api:serialize()
				session:stop()
				return Promise.resolve({
					models = { { id = "GOOD", modelContents = contents("GOOD") } },
					refPatch = PatchSet.newEmpty(),
				})
			end
			expect(session:__replaceInstances({ "GOOD" })).to.equal(false)
			expect(good.Parent ~= nil).to.equal(true)
		end)
	end)

	it("restores the old map binding when a native swap cannot parent its replacement", function()
		fixture(function(session, api, good)
			local insert = session.__instanceMap.insert
			session.__instanceMap.insert = function(map, id, value)
				insert(map, id, value)
				if value ~= good then
					value:Destroy()
				end
			end
			function api:serialize()
				return Promise.resolve({
					models = { { id = "GOOD", modelContents = contents("GOOD") } },
					refPatch = PatchSet.newEmpty(),
				})
			end
			expect(session:__replaceInstances({ "GOOD" })).to.equal(false)
			expect(session.__instanceMap.fromIds.GOOD).to.equal(good)
		end)
	end)

	it("retains original failed updates and reference failures after partial fallback", function()
		fixture(function(session, api)
			local Settings = require(script.Parent.Settings)
			local oldSetting = Settings._values.enableSyncFallback
			Settings._values.enableSyncFallback = true
			local patch = {
				added = {},
				removed = {},
				updated = {
					{ id = "GOOD", changedProperties = { Name = { String = "Good" } } },
					{ id = "BAD", changedProperties = { Name = { String = "Bad" } } },
				},
			}
			local refFailure = { id = "GOOD", changedProperties = { Value = { Ref = "BAD" } } }
			local calls = 0
			session.__reconciler.applyPatch = function()
				calls += 1
				return if calls == 1 then patch else { added = {}, removed = {}, updated = { refFailure } }
			end
			function api:serialize()
				return Promise.resolve({
					models = {
						{ id = "GOOD", modelContents = contents("GOOD", "ObjectValue") },
						{
							id = "BAD",
							error = "bad",
						},
					},
					refPatch = PatchSet.newEmpty(),
				})
			end
			local ok, err = pcall(function()
				session:__applyPatch(patch)
			end)
			Settings._values.enableSyncFallback = oldSetting
			assert(ok, tostring(err))
			expect(#patch.updated).to.equal(2)
			expect(patch.updated[1].id).to.equal("BAD")
			expect(patch.updated[2]).to.equal(refFailure)
		end)
	end)

	it("negotiates isolated serialization and preserves legacy URLs and session validation", function()
		local Http = require(Packages.Http)
		local ApiContext = require(script.Parent.ApiContext)
		local Config = require(script.Parent.Config)
		local oldGet, oldPost, oldMsgpack = Http.get, Http.post, Http.Response.msgpack
		local ok, err = pcall(function()
			for _, modern in { true, false } do
				local context = ApiContext.new("http://test.invalid")
				Http.Response.msgpack = function(response)
					return response.payload
				end
				Http.get = function()
					return Promise.resolve({
						code = 200,
						payload = {
							sessionId = "SESSION",
							protocolVersion = Config.protocolVersion,
							serverVersion = "7.7.1",
							rootInstanceId = "ROOT",
							capabilities = if modern then { "serializedModelsV1" } else {},
						},
					})
				end
				expect(context:connect():await()).to.equal(true)
				expect(context.__serializedModels).to.equal(modern)
				local wrongSession = false
				Http.post = function(url)
					expect(url).to.equal(
						"http://test.invalid/api/serialize" .. if modern then "?isolateModels=true" else ""
					)
					return Promise.resolve({
						code = 200,
						payload = {
							sessionId = if wrongSession then "OTHER" else "SESSION",
							models = {},
							refPatch = PatchSet.newEmpty(),
							modelContents = buffer.create(0),
						},
					})
				end
				expect(context:serialize({ "GOOD" }):await()).to.equal(true)
				wrongSession = true
				expect(context:serialize({ "GOOD" }):await()).to.equal(false)
				context:disconnect()
			end
		end)
		Http.get, Http.post, Http.Response.msgpack = oldGet, oldPost, oldMsgpack
		assert(ok, tostring(err))
	end)
end
