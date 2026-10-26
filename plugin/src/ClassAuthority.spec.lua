return function()
	local ClassAuthority = require(script.Parent.ClassAuthority)
	local InstanceMap = require(script.Parent.InstanceMap)
	local PatchSet = require(script.Parent.PatchSet)
	local ServeSession = require(script.Parent.ServeSession)
	local Promise = require(script.Parent.Parent.Packages.Promise)

	local function fixture(callback)
		local folder = Instance.new("Folder")
		local part = Instance.new("Part")
		part.Transparency = 0.8
		part:SetAttribute("Owner", "Studio")
		part.Parent = folder
		local map = InstanceMap.new()
		map:insert("F", folder)
		map:insert("P", part)
		local authority = ClassAuthority.new(map)
		authority:seed({ F = {}, P = {} }, { F = "filesystem", P = "studio" }, true)
		local ok, err = pcall(callback, authority, map, folder, part)
		map:stop()
		folder:Destroy()
		assert(ok, tostring(err))
	end

	it("preserves existing Studio properties and resets while filesystem controls still apply", function()
		fixture(function(authority, _, _, part)
			local patch = PatchSet.newEmpty()
			patch.updated = {
				{
					id = "P",
					changedProperties = { Transparency = { Float32 = 0.2 } },
					removedProperties = { "Attributes", "Tags" },
				},
				{ id = "F", changedProperties = { Archivable = { Bool = false } } },
			}
			local filtered, review = authority:initial(patch)
			expect(review).to.equal(false)
			expect(next(filtered.updated[1].changedProperties)).to.equal(nil)
			expect(filtered.updated[1].removedProperties).to.equal(nil)
			expect(filtered.updated[2]).to.equal(patch.updated[2])
			expect(patch.updated[1].changedProperties.Transparency.Float32).to.equal(0.2)
			authority:accept(PatchSet.newEmpty())
			patch.classAuthority = { P = "studio", F = "filesystem" }
			local live = authority:prepare(patch)
			expect(next(live.updated[1].changedProperties)).to.equal(nil)
			expect(part.Transparency).to.be.near(0.8)
			expect(part:GetAttribute("Owner")).to.equal("Studio")
		end)
	end)

	it(
		"refuses destructive ancestors and policy revocation then requires reconnect review even for empty patches",
		function()
			fixture(function(authority, map, folder, part)
				authority:accept(PatchSet.newEmpty())
				local removal = PatchSet.newEmpty()
				removal.removed = { "F" }
				expect(authority:prepare(removal)).to.equal(nil)
				local change = PatchSet.newEmpty()
				change.updated = { { id = "F", changedClassName = "Model", changedProperties = {} } }
				change.classAuthority = { F = "filesystem" }
				expect(authority:prepare(change)).to.equal(nil)
				change.updated = { { id = "P", changedProperties = { Transparency = { Float32 = 0.1 } } } }
				change.classAuthority = { P = "filesystem" }
				expect(authority:prepare(change)).to.equal(nil)
				authority:stop()
				local reconnect = ClassAuthority.new(map)
				reconnect:seed({ F = {}, P = {} }, { F = "filesystem", P = "filesystem" }, true)
				local _, review = reconnect:initial(PatchSet.newEmpty())
				expect(review).to.equal(true)
				expect(part.Parent).to.equal(folder)
				expect(part.Transparency).to.be.near(0.8)
				reconnect:accept(PatchSet.newEmpty())
				_, review = reconnect:initial(PatchSet.newEmpty())
				expect(review).to.equal(false)
			end)
		end
	)

	it("rejects malformed policy maps and ambiguous hydration before touching instances", function()
		fixture(function(authority, _, folder, part)
			local records = { F = { Children = { "P" } }, P = { Name = part.Name, ClassName = "Part", Children = {} } }
			authority:validateHydration(records, "F", folder)
			local differentlyTyped = Instance.new("Folder")
			differentlyTyped.Name, differentlyTyped.Parent = part.Name, folder
			records.Q = { Name = part.Name, ClassName = "Folder", Children = {} }
			table.insert(records.F.Children, "Q")
			authority:seed(records, { F = "legacy", P = "studio", Q = "legacy" }, true)
			expect(pcall(authority.validateHydration, authority, records, "F", folder)).to.equal(true)
			local copy = part:Clone()
			copy.Parent = folder
			expect(pcall(authority.validateHydration, authority, records, "F", folder)).to.equal(false)
			expect(pcall(authority.seed, authority, records, { F = "legacy", P = "twoWay" }, true)).to.equal(false)
			expect(part.Parent).to.equal(folder)
			expect(copy.Parent).to.equal(folder)
		end)
	end)

	it("keeps reconstruction review across reconnect when the old instance no longer exists", function()
		fixture(function(authority, map, folder)
			local records, modes = { F = {}, P = {} }, { F = "legacy", P = "studio" }
			local context = { sessionId = "lost-instance-review", root = folder, rootId = "F" }
			authority:seed(records, modes, true, context)
			authority:requireReview()
			authority:stop()
			map:removeId("P")
			local reconnect = ClassAuthority.new(map)
			reconnect:seed(records, modes, true, context)
			local _, review = reconnect:initial(PatchSet.newEmpty())
			expect(review).to.equal(true)
			reconnect:accept(PatchSet.newEmpty())
			local nextConnection = ClassAuthority.new(map)
			nextConnection:seed(records, modes, true, context)
			_, review = nextConnection:initial(PatchSet.newEmpty())
			expect(review).to.equal(false)
		end)
	end)

	it("blocks filesystem model scaling and serialized fallback from discarding Studio fields", function()
		fixture(function(authority, map, folder, part)
			local model = Instance.new("Model")
			model.Parent = folder
			part.Parent = model
			map:insert("M", model)
			authority:seed({ F = {}, M = {}, P = {} }, { F = "filesystem", M = "filesystem", P = "studio" }, true)
			local patch = PatchSet.newEmpty()
			patch.updated = { { id = "M", changedProperties = { Scale = { Float32 = 2 } } } }
			local size = part.Size
			expect(pcall(authority.initial, authority, patch)).to.equal(false)
			expect(part.Size).to.equal(size)
			authority:accept(PatchSet.newEmpty())
			local serialized = false
			local api = {
				disconnect = function() end,
				serialize = function()
					serialized = true
				end,
			}
			local session = ServeSession.new({ apiContext = api, twoWaySync = false })
			session.__instanceMap:insert("M", model)
			session.__instanceMap:insert("P", part)
			local ok, err = pcall(function()
				expect(session:__replaceInstances({ "M" })).to.equal(false)
				expect(serialized).to.equal(false)
				expect(session.__status).to.equal(ServeSession.Status.Disconnected)
				expect(part.Parent).to.equal(model)
				expect(part.Size).to.equal(size)
			end)
			session:stop()
			assert(ok, tostring(err))
		end)
	end)

	it("ServeSession preserves live Studio fields, file scripts, and stops before replacement or revocation", function()
		local folder = Instance.new("Folder")
		folder.Name = "ClassAuthorityAcceptance"
		folder.Parent = game.ServerStorage
		local part = Instance.new("Part")
		part.Name, part.Transparency, part.Parent = "Pole", 0.8, folder
		local records = {
			ROOT = {
				Name = game.Name,
				ClassName = "DataModel",
				Properties = {},
				Children = { "SS" },
				Metadata = { ignoreUnknownInstances = true },
			},
			SS = {
				Name = "ServerStorage",
				ClassName = "ServerStorage",
				Parent = "ROOT",
				Properties = {},
				Children = { "F" },
				Metadata = { ignoreUnknownInstances = true },
			},
			F = {
				Name = folder.Name,
				ClassName = "Folder",
				Parent = "SS",
				Properties = {},
				Children = { "P", "S" },
				Metadata = { ignoreUnknownInstances = true },
			},
			P = {
				Name = "Pole",
				ClassName = "Part",
				Parent = "F",
				Properties = { Transparency = { Float32 = 0.2 } },
				Children = {},
			},
			S = {
				Name = "Script",
				ClassName = "ModuleScript",
				Parent = "F",
				Properties = { Source = { String = "return 1" } },
				Children = {},
			},
		}
		local modes = { ROOT = "legacy", SS = "legacy", F = "legacy", P = "studio", S = "filesystem" }
		local handlers, finish
		local api = {}
		function api:connect()
			return Promise.resolve({
				sessionId = "class-authority-unit",
				rootInstanceId = "ROOT",
				projectName = "Authority",
				capabilities = { "classAuthorityV1" },
			})
		end
		function api:read()
			return Promise.resolve({ instances = records, classAuthority = modes, messageCursor = 0 })
		end
		function api:setMessageCursor(cursor)
			self.cursor = cursor
		end
		function api:connectWebSocket(callbacks)
			handlers = callbacks
			return Promise.new(function(resolve)
				finish = resolve
			end)
		end
		function api:disconnect()
			if finish then
				finish()
			end
		end
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		local ok, err = pcall(function()
			session:start()
			local deadline = os.clock() + 4
			repeat
				task.wait()
			until handlers or session.__status == ServeSession.Status.Disconnected or os.clock() > deadline
			expect(session.__status).to.equal(ServeSession.Status.Connected)
			expect(session.__instanceMap.fromIds.P).to.equal(part)
			expect(part.Transparency).to.be.near(0.8)
			local source = session.__instanceMap.fromIds.S
			expect(source.Source).to.equal("return 1")
			local patch = PatchSet.newEmpty()
			patch.updated = {
				{ id = "P", changedProperties = { Transparency = { Float32 = 0.3 } }, removedProperties = { "Tags" } },
				{ id = "S", changedProperties = { Source = { String = "return 2" } } },
			}
			patch.classAuthority = { P = "studio", S = "filesystem" }
			handlers.messages({ messageCursor = 1, messages = { patch } })
			expect(part.Transparency).to.be.near(0.8)
			expect(source.Source).to.equal("return 2")
			expect(api.cursor).to.equal(1)
			patch.classAuthority.P = "filesystem"
			handlers.messages({ messageCursor = 2, messages = { patch } })
			expect(session.__status).to.equal(ServeSession.Status.Disconnected)
			expect(part.Transparency).to.be.near(0.8)
			expect(part.Parent).to.equal(folder)
			expect(api.cursor).to.equal(1)
		end)
		session:stop()
		folder:Destroy()
		assert(ok, tostring(err))
	end)
end
