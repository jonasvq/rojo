return function()
	local Packages = script.Parent.Parent.Packages
	local Promise = require(Packages.Promise)
	local ServeSession = require(script.Parent.ServeSession)
	local Settings = require(script.Parent.Settings)
	local SourceWrites = require(script.Parent.SourceWrites)

	local function startSession(info, instances, decision)
		local finishSocket
		local handlers
		local api = {}
		local cursors = {}
		function api:connect()
			return Promise.resolve(info)
		end
		function api:read()
			return Promise.resolve({ instances = instances, messageCursor = 0 })
		end
		function api:setMessageCursor(cursor)
			table.insert(cursors, cursor)
		end
		function api:connectWebSocket(callbacks)
			handlers = callbacks
			return Promise.new(function(resolve)
				finishSocket = resolve
			end)
		end
		function api:disconnect()
			if finishSocket then
				finishSocket()
			end
		end
		local session = ServeSession.new({ apiContext = api, twoWaySync = false })
		if decision then
			session:setConfirmCallback(function()
				return decision
			end)
		end
		local status, detail
		session:onStatusChanged(function(value, message)
			status, detail = value, message
			if value == ServeSession.Status.Disconnected and type(message) == "table" and message.getErrorChain then
				for _, cause in message:getErrorChain() do
					print("RECONNECT_TEST_CAUSE " .. tostring(cause.error))
				end
			end
		end)
		session:start()
		local deadline = os.clock() + 5
		repeat
			task.wait()
		until status == ServeSession.Status.Connected
			or status == ServeSession.Status.Disconnected
			or os.clock() > deadline
		local expected = if decision == "Abort" then ServeSession.Status.Disconnected else ServeSession.Status.Connected
		if status ~= expected then
			session:stop()
			error(tostring(detail or status))
		end
		return session, function(packet)
			handlers.messages(packet)
		end, cursors
	end

	describe("managed topology", function()
		local function check(mode, adopt)
			local previous = Settings:get("adoptMatchingInstances")
			Settings:set("adoptMatchingInstances", adopt)
			local session, root, movedParent, original, copy
			local ok, reason = pcall(function()
				local instances = {
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
						Children = { "TEST" },
						Metadata = { ignoreUnknownInstances = true },
					},
					TEST = {
						Name = "TopologyTest",
						ClassName = "Folder",
						Parent = "SS",
						Properties = {},
						Children = { "PART" },
						Metadata = { ignoreUnknownInstances = true },
					},
					PART = {
						Name = "Pole",
						ClassName = "Part",
						Parent = "TEST",
						Properties = { Transparency = { Float32 = 0 }, Anchored = { Bool = true } },
						Children = {},
						Metadata = { ignoreUnknownInstances = true },
					},
				}
				local deliver
				session, deliver = startSession(
					{ sessionId = "topology-" .. mode, rootInstanceId = "ROOT", projectName = "Topology" },
					instances,
					"Accept"
				)
				root, original = session.__instanceMap.fromIds.TEST, session.__instanceMap.fromIds.PART
				if mode == "move" or mode == "paused-move" or mode == "undo-move" then
					movedParent = Instance.new("Folder")
					movedParent.Parent = game.ServerStorage
					if mode == "paused-move" then
						session.__instanceMap:pauseInstance(original)
					end
					local history = game:GetService("ChangeHistoryService")
					local manual = if mode == "undo-move"
						then history:TryBeginRecording("Topology test manual move")
						else nil
					if mode == "undo-move" then
						assert(manual, "manual move history control unavailable")
					end
					original.Parent = movedParent
					if manual then
						history:FinishRecording(manual, Enum.FinishRecordingOperation.Commit)
					end
					if mode == "paused-move" or mode == "undo-move" then
						local deadline = os.clock() + 3
						repeat
							task.wait()
						until original.Parent == root or os.clock() > deadline
						assert(original.Parent == root, "property pause hid an idle ancestry repair")
					end
					if mode == "undo-move" then
						history:Undo()
						task.wait(0.1)
						assert(
							original.Parent == movedParent and session.__status == ServeSession.Status.Connected,
							"idle recovery immediately undid the user's Undo"
						)
					end
				else
					if mode == "copy" or mode == "refuse" then
						copy = original:Clone()
						copy.Parent = root
					end
					original:Destroy()
				end
				instances.PART.Properties.Transparency = { Float32 = 0.4 }
				deliver({
					messageCursor = 1,
					messages = {
						{
							removed = {},
							added = {},
							updated = { { id = "PART", changedProperties = { Transparency = { Float32 = 0.4 } } } },
						},
					},
				})
				local deadline = os.clock() + 3
				repeat
					task.wait()
				until session.__status == ServeSession.Status.Disconnected
					or (session.__instanceMap.fromIds.PART and session.__instanceMap.fromIds.PART.Parent == root)
					or os.clock() > deadline
				if mode == "refuse" then
					assert(
						session.__status == ServeSession.Status.Disconnected,
						"unapproved replacement kept ghost mapping connected"
					)
					assert(
						copy.Parent == root and copy.Transparency == 0 and #root:GetChildren() == 1,
						"refusal changed or duplicated the copy"
					)
					assert(original.Transparency == 0, "refusal mutated detached original")
				else
					assert(
						session.__status == ServeSession.Status.Connected,
						"unambiguous topology repair disconnected"
					)
					local mapped = session.__instanceMap.fromIds.PART
					assert(
						mapped and mapped.Parent == root and #root:GetChildren() == 1,
						"mapped source remained detached or duplicated"
					)
					assert(
						math.abs(mapped.Transparency - 0.4) < 0.001,
						"repaired visible source missed incoming update"
					)
					if mode == "move" or mode == "paused-move" or mode == "undo-move" then
						assert(mapped == original, "move repair replaced identity")
					end
					if mode == "copy" then
						assert(mapped == copy, "copy recovery did not retain the permitted candidate")
					end
				end
			end)
			Settings:set("adoptMatchingInstances", previous)
			if session then
				session:stop()
			end
			if root then
				root:Destroy()
			end
			if movedParent then
				movedParent:Destroy()
			end
			assert(ok, tostring(reason))
		end
		it("repairs a moved original before applying an incoming update", function()
			check("move", false)
		end)
		it("observes idle ancestry while property callbacks remain paused", function()
			check("paused-move", false)
		end)
		it("preserves Undo until a later authoritative patch rechecks topology", function()
			check("undo-move", false)
		end)
		it("rehydrates an uncontested deleted mapped instance", function()
			check("delete", false)
		end)
		it("rebinds an exact copy only under the adoption setting", function()
			check("copy", true)
		end)
		it("refuses an unapproved copy before updating its detached original", function()
			check("refuse", false)
		end)
		it("retains accepted engine side effects when recovering a scaled model copy", function()
			local previous = Settings:get("adoptMatchingInstances")
			Settings:set("adoptMatchingInstances", true)
			local session, original, replacement
			local ok, reason = pcall(function()
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
						Children = { "TEST" },
						Metadata = { ignoreUnknownInstances = true },
					},
					TEST = {
						Name = "TopologyScaleTest",
						ClassName = "Model",
						Parent = "SS",
						Properties = { Scale = { Float32 = 1 } },
						Children = { "PART" },
						Metadata = { ignoreUnknownInstances = true },
					},
					PART = {
						Name = "Pole",
						ClassName = "Part",
						Parent = "TEST",
						Properties = { Size = { Vector3 = { 1, 1, 1 } }, Anchored = { Bool = true } },
						Children = {},
						Metadata = { ignoreUnknownInstances = true },
					},
				}
				local deliver
				session, deliver = startSession(
					{ sessionId = "topology-scale-copy", rootInstanceId = "ROOT", projectName = "TopologyScale" },
					records,
					"Accept"
				)
				original = session.__instanceMap.fromIds.TEST
				deliver({
					messageCursor = 1,
					messages = {
						{
							removed = {},
							added = {},
							updated = { { id = "TEST", changedProperties = { Scale = { Float32 = 3 } } } },
						},
					},
				})
				assert(original.Pole.Size == Vector3.new(3, 3, 3), "control did not scale the assembled model")
				replacement = original:Clone()
				replacement.Parent = original.Parent
				original:Destroy()
				local deadline = os.clock() + 3
				repeat
					task.wait()
				until session.__status == ServeSession.Status.Disconnected
					or session.__instanceMap.fromIds.TEST == replacement
					or os.clock() > deadline
				assert(
					session.__status == ServeSession.Status.Connected
						and session.__instanceMap.fromIds.TEST == replacement,
					"accepted scale side effects made an exact copy appear different"
				)
				assert(
					session.__instanceMap.fromIds.PART == replacement.Pole
						and replacement.Pole.Size == Vector3.new(3, 3, 3),
					"scaled child identity/value lost"
				)
			end)
			Settings:set("adoptMatchingInstances", previous)
			if session then
				session:stop()
			end
			if original then
				original:Destroy()
			end
			if replacement then
				replacement:Destroy()
			end
			assert(ok, tostring(reason))
		end)
		it("requires reconnect review for a copy containing unconfirmed descendant Source", function()
			local previous = Settings:get("adoptMatchingInstances")
			Settings:set("adoptMatchingInstances", true)
			local session, reconnect, root, candidate
			local ok, reason = pcall(function()
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
						Children = { "TEST" },
						Metadata = { ignoreUnknownInstances = true },
					},
					TEST = {
						Name = "TopologyReviewTest",
						ClassName = "Part",
						Parent = "SS",
						Properties = {},
						Children = { "SCRIPT" },
						Metadata = { ignoreUnknownInstances = true },
					},
					SCRIPT = {
						Name = "Source",
						ClassName = "ModuleScript",
						Parent = "TEST",
						Properties = { Source = { String = "return 1" } },
						Children = {},
						Metadata = { ignoreUnknownInstances = true },
					},
				}
				session = startSession(
					{ sessionId = "topology-dirty-copy", rootInstanceId = "ROOT", projectName = "TopologyReview" },
					records,
					"Accept"
				)
				root = session.__instanceMap.fromIds.TEST
				local api = session.__apiContext
				function api:sourceView()
					return Promise.resolve({
						source = "return 1",
						className = "ModuleScript",
						sourceHash = "base",
						bindingHash = "binding",
					})
				end
				function api:submitSource()
					return Promise.resolve({ receiptId = "pending" })
				end
				function api:sourceReceipt()
					return Promise.new(function() end)
				end
				session.__sourceWrites = SourceWrites.new(api, session.__instanceMap)
				session.__sourceWrites:seed(
					records,
					{ SCRIPT = { className = "ModuleScript", sourceHash = "base", bindingHash = "binding" } }
				)
				session.__twoWaySync = true
				root.Source.Source = "return 2"
				local deadline = os.clock() + 3
				repeat
					task.wait()
				until session.__sourceWrites.states.SCRIPT.running or os.clock() > deadline
				assert(session.__sourceWrites.states.SCRIPT.running, "pending Source control never started")
				candidate = root:Clone()
				candidate.Parent = root.Parent
				root:Destroy()
				deadline = os.clock() + 3
				repeat
					task.wait()
				until session.__status == ServeSession.Status.Disconnected or os.clock() > deadline
				assert(
					session.__status == ServeSession.Status.Disconnected and candidate.Source.Source == "return 2",
					"dirty copy was changed instead of refused"
				)
				assert(
					SourceWrites.needsReview(),
					"destroying the original lost recovery review for its live dirty copy"
				)
				reconnect = ServeSession.new({ apiContext = api, twoWaySync = false })
				reconnect:start()
				deadline = os.clock() + 3
				repeat
					task.wait()
				until reconnect.__status ~= ServeSession.Status.Connecting or os.clock() > deadline
				assert(
					reconnect.__status == ServeSession.Status.Disconnected and candidate.Source.Source == "return 2",
					"reconnect bypassed dirty-copy confirmation"
				)
			end)
			Settings:set("adoptMatchingInstances", previous)
			if session then
				session:stop()
			end
			if reconnect then
				reconnect:stop()
			end
			if candidate then
				SourceWrites.clearReviewed({ candidate })
				candidate:Destroy()
			end
			if root then
				root:Destroy()
			end
			assert(ok, tostring(reason))
		end)
	end)

	it("stops a refused adoption batch without fallback or cursor advancement", function()
		local previous = Settings:get("adoptMatchingInstances")
		Settings:set("adoptMatchingInstances", true)
		local session, root
		local ok, reason = pcall(function()
			local instances = {
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
					Children = { "TEST" },
					Metadata = { ignoreUnknownInstances = true },
				},
				TEST = {
					Name = "AdoptionConflictTest",
					ClassName = "Folder",
					Parent = "SS",
					Properties = {},
					Children = {},
					Metadata = { ignoreUnknownInstances = true },
				},
			}
			local deliver, cursors
			session, deliver, cursors = startSession(
				{ sessionId = "adoption-refusal", rootInstanceId = "ROOT", projectName = "Adoption" },
				instances,
				"Accept"
			)
			root = session.__instanceMap.fromIds.TEST
			local original = Instance.new("Part")
			original.Name, original.Anchored, original.Parent = "Saved", false, root
			deliver({
				messageCursor = 99,
				messages = {
					{
						removed = {},
						updated = {},
						added = {
							PART = {
								Id = "PART",
								Parent = "TEST",
								Name = "Saved",
								ClassName = "Part",
								Properties = { Anchored = { Bool = true } },
								Children = {},
							},
						},
					},
				},
			})
			assert(session.__status == ServeSession.Status.Disconnected, "adoption conflict did not stop the session")
			assert(
				#root:GetChildren() == 1 and root:FindFirstChild("Saved") == original and not original.Anchored,
				("refusal state children=%d original=%s parent=%s anchored=%s"):format(
					#root:GetChildren(),
					original.Name,
					tostring(original.Parent),
					tostring(original.Anchored)
				)
			)
			assert(cursors[#cursors] == 0, "refused patch advanced the subscription cursor")
		end)
		Settings:set("adoptMatchingInstances", previous)
		if session then
			session:stop()
		end
		if root then
			root:Destroy()
		end
		assert(ok, tostring(reason))
	end)
	it("keeps initial review separate from live adoption collision checks", function()
		local previous = Settings:get("adoptMatchingInstances")
		Settings:set("adoptMatchingInstances", true)
		local original = Instance.new("Folder")
		original.Name, original.Parent = "InitialAdoptionReview", game:GetService("ServerStorage")
		local session
		local ok, reason = pcall(function()
			local instances = {
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
					Children = { "TEST" },
					Metadata = { ignoreUnknownInstances = true },
				},
				TEST = {
					Name = "InitialAdoptionReview",
					ClassName = "Part",
					Parent = "SS",
					Properties = { Anchored = { Bool = true } },
					Children = {},
				},
			}
			session = startSession(
				{ sessionId = "initial-adoption-review", rootInstanceId = "ROOT", projectName = "Adoption" },
				instances,
				"Accept"
			)
			assert(
				session.__instanceMap.fromIds.TEST:IsA("Part"),
				"initial approved addition was blocked by live adoption policy"
			)
			assert(original.Parent == game.ServerStorage, "initial preservation changed unknown object")
		end)
		Settings:set("adoptMatchingInstances", previous)
		if session then
			local managed = session.__instanceMap.fromIds.TEST
			session:stop()
			if managed then
				managed:Destroy()
			end
		end
		original:Destroy()
		assert(ok, tostring(reason))
	end)

	local function snapshot(name, className)
		local instances = {
			ROOT = {
				ClassName = "DataModel",
				Name = game.Name,
				Properties = {},
				Children = { "SP" },
				Metadata = { ignoreUnknownInstances = true },
			},
			SP = {
				ClassName = "StarterPlayer",
				Name = "StarterPlayer",
				Properties = {},
				Children = { "SPS" },
				Metadata = { ignoreUnknownInstances = true },
			},
			SPS = {
				ClassName = "StarterPlayerScripts",
				Name = "StarterPlayerScripts",
				Properties = {},
				Children = { "CLIENT" },
				Metadata = { ignoreUnknownInstances = true },
			},
			CLIENT = { ClassName = className, Name = name, Properties = {}, Children = { "TEST" } },
			TEST = { ClassName = "Folder", Name = "test", Properties = {}, Children = { "A" } },
			A = { ClassName = "ModuleScript", Name = "a", Properties = {}, Children = {} },
		}
		for id, instance in instances do
			instance.Id = id
			for _, childId in instance.Children do
				instances[childId].Parent = id
			end
		end
		return instances
	end

	it("produces the same script-to-folder result while connected and after reconnecting", function()
		local scripts = game:GetService("StarterPlayer"):FindFirstChildOfClass("StarterPlayerScripts")
		assert(scripts, "Test place must contain StarterPlayerScripts")
		for _, disconnectDuringChange in { false, true } do
			local name = "Rojo1009" .. tostring(disconnectDuringChange)
			local original = Instance.new("LocalScript")
			original.Name = name
			original.Parent = scripts
			local test = Instance.new("Folder")
			test.Name = "test"
			test.Parent = original
			local module = Instance.new("ModuleScript")
			module.Name = "a"
			module.Parent = test
			local sessions = {}
			local ok, err = pcall(function()
				local info = { sessionId = name, rootInstanceId = "ROOT", projectName = "Reconnect" }
				local first, deliver = startSession(info, snapshot(name, "LocalScript"))
				table.insert(sessions, first)
				if disconnectDuringChange then
					first:stop()
					local second = startSession(info, snapshot(name, "Folder"))
					table.insert(sessions, second)
				else
					deliver({
						messageCursor = 1,
						messages = {
							{
								removed = {},
								added = {},
								updated = {
									{
										id = "CLIENT",
										changedClassName = "Folder",
										changedProperties = {},
									},
								},
							},
						},
					})
				end
				local matches = {}
				for _, child in scripts:GetChildren() do
					if child.Name == name then
						table.insert(matches, child)
					end
				end
				assert(
					#matches == 1,
					("Expected one Client after reconnect=%s, got %d"):format(
						tostring(disconnectDuringChange),
						#matches
					)
				)
				expect(matches[1].ClassName).to.equal("Folder")
				expect(matches[1].test.a).to.equal(module)
			end)
			for _, session in sessions do
				session:stop()
			end
			for _, child in scripts:GetChildren() do
				if child.Name == name then
					child:Destroy()
				end
			end
			original:Destroy()
			assert(ok, tostring(err))
		end
	end)

	it("does not overwrite accepted ownership when another connection is aborted", function()
		local scripts = game:GetService("StarterPlayer"):FindFirstChildOfClass("StarterPlayerScripts")
		local name = "Rojo1009Abort"
		local original = Instance.new("LocalScript")
		original.Name = name
		original.Parent = scripts
		local test = Instance.new("Folder")
		test.Name = "test"
		test.Parent = original
		local module = Instance.new("ModuleScript")
		module.Name = "a"
		module.Parent = test
		local sessions = {}
		local ok, err = pcall(function()
			local info = { sessionId = name, rootInstanceId = "ROOT", projectName = "Reconnect" }
			local first = startSession(info, snapshot(name, "LocalScript"))
			table.insert(sessions, first)
			first:stop()
			local aborted = startSession(
				{ sessionId = "different-server", rootInstanceId = "ROOT", projectName = "Abort" },
				snapshot(name, "Folder"),
				"Abort"
			)
			table.insert(sessions, aborted)
			local last = startSession(info, snapshot(name, "Folder"))
			table.insert(sessions, last)
			local matches = {}
			for _, child in scripts:GetChildren() do
				if child.Name == name then
					table.insert(matches, child)
				end
			end
			expect(#matches).to.equal(1)
			expect(matches[1].ClassName).to.equal("Folder")
			expect(matches[1].test.a).to.equal(module)
		end)
		for _, session in sessions do
			session:stop()
		end
		for _, child in scripts:GetChildren() do
			if child.Name == name then
				child:Destroy()
			end
		end
		original:Destroy()
		assert(ok, tostring(err))
	end)
end
