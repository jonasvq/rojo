return function()
	local Promise = require(script.Parent.Parent.Packages.Promise)
	local SourceWrites = require(script.Parent.SourceWrites)
	local function waitFor(predicate)
		local deadline = os.clock() + 3
		repeat
			task.wait()
		until predicate() or os.clock() > deadline
		assert(predicate(), "source receipt condition timed out")
	end
	local function fixture()
		local instance = Instance.new("ModuleScript")
		instance.Name = "ReceiptSource"
		instance.Source = "base"
		local map =
			{ fromIds = { ID = instance }, fromInstances = { [instance] = "ID" }, pauseInstance = function() end }
		local api = { source = "base", submissions = {}, replies = {}, reads = 0 }
		function api:sourceView()
			self.reads += 1
			return Promise.resolve({
				source = self.source,
				className = "ModuleScript",
				sourceHash = self.source .. "-hash",
				bindingHash = "binding",
				sourcePath = "fixture.luau",
			})
		end
		function api:submitSource(request)
			table.insert(self.submissions, request)
			return Promise.resolve({ receiptId = tostring(#self.submissions) })
		end
		function api:sourceReceipt(id)
			return Promise.new(function(resolve)
				self.replies[id] = resolve
			end)
		end
		function api:finish(id, source)
			self.source = source
			self.replies[tostring(id)]({ state = "applied", sourceHash = source .. "-hash", bindingHash = "binding" })
		end
		local controller = SourceWrites.new(api, map)
		controller:seed(
			{ ID = { ClassName = "ModuleScript", Properties = { Source = { String = "base" } } } },
			{ ID = { className = "ModuleScript", sourceHash = "base-hash", bindingHash = "binding" } }
		)
		return controller, api, instance
	end
	describe("SourceWrites", function()
		it("retires wide clean removals with bounded mapping work", function()
			local data, instances, removed, records = {}, {}, {}, {}
			for index = 1, 1000 do
				local id = "SOURCE_" .. index
				local instance = { Name = id, Parent = nil }
				data[id], instances[instance] = instance, id
				table.insert(removed, id)
				records[id] = { ClassName = "ModuleScript", Properties = { Source = { String = "base" } } }
			end
			local lookups = 0
			local map = {
				fromInstances = instances,
				fromIds = setmetatable({}, {
					__index = function(_, id)
						lookups += 1
						return data[id]
					end,
				}),
			}
			local controller = SourceWrites.new({}, map)
			controller:seed(records, {})
			assert(controller:prepare({ removed = removed, updated = {}, added = {} }))
			assert(lookups <= 4000, "wide source removal repeatedly scanned all bindings: " .. lookups)
			assert(next(controller.states) == nil, "removed clean sources retained their old baselines")
			controller:stop()
		end)
		it("coalesces a newer edit and preserves it across echo-before-receipt ordering", function()
			local controller, api, instance = fixture()
			instance.Source = "first"
			controller:edit("ID", "first")
			waitFor(function()
				return api.replies["1"] ~= nil
			end)
			instance.Source = "second"
			controller:edit("ID", "second")
			local patch = {
				added = {},
				removed = {},
				updated = {
					{ id = "ID", changedProperties = { Source = { String = "first" }, Archivable = { Bool = false } } },
				},
			}
			assert(controller:prepare(patch))
			assert(
				patch.updated[1].changedProperties.Source == nil and patch.updated[1].changedProperties.Archivable,
				"echo must preserve newer Source and unrelated properties"
			)
			assert(instance.Source == "second", "newer source was overwritten")
			api:finish(1, "first")
			waitFor(function()
				return api.replies["2"] ~= nil
			end)
			assert(
				api.submissions[2].source == "second" and api.submissions[2].sourceHash == "first-hash",
				"newer edit did not use the confirmed base"
			)
			api:finish(2, "second")
			waitFor(function()
				return controller:summary().pending == 0
			end)
			assert(#controller:summary().issues == 0, "successful writes left a conflict")
			controller:stop()
			instance:Destroy()
		end)
		it("keeps local source on a different remote value and requires explicit resolution", function()
			local controller, api, instance = fixture()
			instance.Source = "local"
			controller:edit("ID", "local")
			waitFor(function()
				return api.replies["1"] ~= nil
			end)
			local patch = {
				added = {},
				removed = {},
				updated = { { id = "ID", changedProperties = { Source = { String = "remote" } } } },
			}
			assert(controller:prepare(patch))
			assert(
				patch.updated[1].changedProperties.Source == nil and instance.Source == "local",
				"remote source overwrote the local edit"
			)
			assert(#controller:summary().issues == 1, "remote conflict was not surfaced")
			api:finish(1, "remote")
			waitFor(function()
				return controller:summary().pending == 0
			end)
			assert(
				#controller:summary().issues == 1 and #api.submissions == 1,
				"late receipt cleared or retried a conflict"
			)
			controller:resolve("ID", "file"):expect()
			assert(instance.Source == "remote" and #controller:summary().issues == 0, "explicit file resolution failed")
			controller:stop()
			instance:Destroy()
		end)
		it("does not silently retry a lost receipt and remembers unresolved edits on stop", function()
			local controller, api, instance = fixture()
			function api:submitSource()
				return Promise.reject("lost response")
			end
			instance.Parent = game:GetService("ServerStorage")
			instance.Source = "local"
			controller:edit("ID", "local")
			waitFor(function()
				return #controller:summary().issues == 1
			end)
			controller:edit("ID", "newer")
			assert(instance.Source == "local", "failed receipt modified Studio Source")
			controller:stop()
			assert(SourceWrites.needsReview())
			instance:Destroy()
		end)
		it("rejects destructive structural patches before applying or advancing the cursor", function()
			local controller, api, instance = fixture()
			instance.Source = "local"
			controller:edit("ID", "local")
			waitFor(function()
				return api.replies["1"] ~= nil
			end)
			local allowed, reason = controller:prepare({ added = {}, removed = { "ID" }, updated = {} })
			assert(not allowed and string.find(reason, "source", 1, true), "dirty source deletion was allowed")
			assert(instance.Source == "local", "structural rejection lost local source")
			controller:stop()
			api:finish(1, "local")
			task.wait()
			instance:Destroy()
		end)
		it("requires a new review if either file or Studio Source changed after review", function()
			local controller, api, instance = fixture()
			function api:submitSource()
				return Promise.reject("uncertain write")
			end
			instance.Source = "local"
			controller:edit("ID", "local")
			waitFor(function()
				return #controller:summary().issues == 1 and controller:summary().pending == 0
			end)
			controller:review("ID"):expect()
			api.source = "remote"
			local applied = controller:resolve("ID", "studio"):await()
			assert(not applied and instance.Source == "local", "stale file review was accepted")
			controller:review("ID"):expect()
			instance.Source = "newer local"
			controller:edit("ID", "newer local")
			assert(not controller:summary().issues[1].reviewed, "local edit did not invalidate the reviewed pair")
			applied = controller:resolve("ID", "studio"):await()
			assert(not applied and instance.Source == "newer local", "unreviewed Studio Source was written")
			controller:stop()
			instance:Destroy()
		end)
		it("cancels a pending source read and never submits after stop", function()
			local controller, api, instance = fixture()
			local reply, cancelled = nil, 0
			function api:sourceView()
				return Promise.new(function(resolve, _, onCancel)
					reply = resolve
					onCancel(function()
						cancelled += 1
					end)
				end)
			end
			instance.Source = "local"
			controller:edit("ID", "local")
			waitFor(function()
				return reply ~= nil
			end)
			controller:stop()
			waitFor(function()
				return cancelled == 1
			end)
			reply({ source = "base", className = "ModuleScript", sourceHash = "base-hash", bindingHash = "binding" })
			task.wait()
			assert(#api.submissions == 0 and instance.Source == "local", "late source read submitted after stop")
			instance:Destroy()
		end)
	end)
end
