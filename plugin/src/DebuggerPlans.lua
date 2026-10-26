local RunService = game:GetService("RunService")
local HttpService = game:GetService("HttpService")
local Native = require(script.Parent.DebuggerNative)

local DebuggerPlans = {}
DebuggerPlans.__index = DebuggerPlans

function DebuggerPlans.supported(session)
	local info = session and session.__serverInfo
	return info and type(info.capabilities) == "table" and table.find(info.capabilities, "debuggerPlansV1") ~= nil
end

function DebuggerPlans.new(session, changed, serviceFactory)
	assert(DebuggerPlans.supported(session), "Debugger plans not advertised")
	return setmetatable({
		session = session,
		changed = changed,
		factory = serviceFactory or function()
			return game:GetService("ScriptDebuggerService")
		end,
		queue = {},
		overflow = 0,
		seen = {},
		states = {},
		enabled = false,
		generation = 0,
		state = "Disabled",
		root = session.__serverInfo.rootInstanceId,
	}, DebuggerPlans)
end

function DebuggerPlans:__state(state)
	self.state = state
	self.changed(state)
end

function DebuggerPlans:__enqueue(desired, report)
	if report.hit >= desired.plan.maxHits and self.states[desired.source.instanceId] then
		self.states[desired.source.instanceId].state = "Completed"
	end
	if #self.queue == 64 then
		table.remove(self.queue, 1)
		self.overflow += 1
	end
	report.overflow = self.overflow
	table.insert(self.queue, {
		operation = "report",
		relayId = self.relayId,
		instanceId = desired.source.instanceId,
		sourceHash = desired.source.sourceHash,
		generation = desired.generation,
		report = report,
	})
end

function DebuggerPlans:__mode()
	if not RunService:IsRunning() then
		return "Edit"
	end
	if RunService:IsServer() and RunService:IsClient() then
		return "Run"
	end
	return "Unsupported"
end

function DebuggerPlans:__clear()
	if self.native then
		self.native:stop()
	end
	table.clear(self.seen)
	table.clear(self.states)
	table.clear(self.queue)
end

function DebuggerPlans:__reconcile(response)
	assert(response.rootId == self.root, "Debugger root changed")
	local mode = self:__mode()
	if mode ~= "Run" then
		self:__clear()
		self:__state(
			if mode == "Edit"
				then "Queued in Edit; points arm only in combined Run"
				else "Separate Play context unsupported"
		)
		return
	end
	if not self.native then
		self.native = Native.new(self.factory(), function(desired, report)
			self:__enqueue(desired, report)
		end, function(id)
			if self.states[id] then
				self.states[id].state = "Invalidated"
			end
			for i = #self.queue, 1, -1 do
				if self.queue[i].instanceId == id then
					table.remove(self.queue, i)
				end
			end
			self:__state("Native source changed; submit a new source version")
		end)
	end
	self.native:install()
	local wanted = {}
	for _, desired in response.plans do
		local source = desired.source
		local id = source.instanceId
		wanted[id] = true
		local active = self.native.entries[id]
		if active and self.session.__instanceMap.fromIds[id] ~= active.instance then
			self.native:remove(id)
			if self.states[id] then
				self.states[id].state = "Invalidated"
			end
			for i = #self.queue, 1, -1 do
				if self.queue[i].instanceId == id then
					table.remove(self.queue, i)
				end
			end
		end
		if self.seen[id] ~= desired.generation then
			self.native:remove(id)
			self.seen[id] = desired.generation
			local state = { instanceId = id, generation = desired.generation, state = "Rejected", points = {} }
			self.states[id] = state
			local instance = self.session.__instanceMap.fromIds[id]
			local ok, err = pcall(function()
				assert(Native.validate(desired.plan), "Invalid debugger plan")
				assert(
					instance
						and instance.Parent
						and instance.Source == source.source
						and instance.ClassName == source.className,
					"Managed native source differs"
				)
				local path = instance:GetFullName()
				local aliases = 0
				for _, other in self.session.__instanceMap.fromIds do
					if other:IsA("LuaSourceContainer") and other:GetFullName() == path then
						aliases += 1
					end
				end
				assert(aliases == 1, "Ambiguous native script path")
				local entry = self.native:arm(desired, instance)
				for verified, requested in entry.lines do
					table.insert(state.points, { requested = requested, verified = verified })
				end
				state.state = "Armed"
			end)
			if not ok then
				state.error = tostring(err):sub(1, 512)
				self:__state("Plan rejected: " .. tostring(err):sub(1, 256))
			end
		end
	end
	for id in self.seen do
		if not wanted[id] then
			self.native:remove(id)
			self.seen[id] = nil
			self.states[id] = nil
		end
	end
	if self.native.diagnostics.error then
		for id, state in self.states do
			if state.state == "Armed" and not self.native.entries[id] then
				state.state = "Rejected"
				state.error = self.native.diagnostics.error
			end
		end
		self:__state("Native debugger guard: " .. self.native.diagnostics.error)
	else
		self:__state("Combined Run; exclusive callback installed by this relay")
	end
end

function DebuggerPlans:__request(body)
	local request = self.session.__apiContext:__debuggerRelay(body, self.relayId)
	self.request = request
	return request
end

function DebuggerPlans:__schedule(generation)
	if not self.enabled or generation ~= self.generation then
		return
	end
	self.timer = task.delay(0.1, function()
		self.timer = nil
		self:__tick(generation)
	end)
end

function DebuggerPlans:__tick(generation)
	if not self.enabled or generation ~= self.generation then
		return
	end
	local session = self.session
	if session.__serverInfo.rootInstanceId ~= self.root or not session.__apiContext.__socketOpen then
		self:stop()
		return
	end
	local body = table.remove(self.queue, 1)
	local operation = if body
		then self:__request(body)
		else self:__request(nil):andThen(function(response)
			if not self.enabled or generation ~= self.generation then
				return
			end
			self:__reconcile(response)
			local states = {}
			for _, state in self.states do
				table.insert(states, state)
			end
			return self:__request({
				operation = "status",
				relayId = self.relayId,
				mode = self:__mode(),
				installed = self.native ~= nil and self.native.installed,
				plans = states,
			})
		end)
	operation
		:andThen(function()
			if not self.enabled or generation ~= self.generation then
				return
			end
			self.request = nil
			self.lastAck = os.clock()
			self:__schedule(generation)
		end)
		:catch(function(err)
			if not self.enabled or generation ~= self.generation then
				return
			end
			self:stop()
			self:__state("Debugger relay stopped: " .. tostring(err):sub(1, 256))
		end)
end

-- Called only by the Connected panel's explicit nonpersistent consent action.
function DebuggerPlans:enable()
	if self.enabled then
		return
	end
	assert(self.session.__apiContext.__socketOpen, "Wait for initial sync and socket readiness")
	self.enabled = true
	self.generation += 1
	local generation = self.generation
	self.relayId = HttpService:GenerateGUID(false)
	self.lastAck = os.clock()
	self.lastMode = self:__mode()
	self:__state("Requesting exclusive debugger relay")
	self.modeConnection = RunService.Heartbeat:Connect(function()
		if not self.enabled then
			return
		end
		local mode = self:__mode()
		if self.lastMode == "Run" and mode ~= "Run" then
			self:stop()
			return
		end
		self.lastMode = mode
		-- Native points cannot survive an unresponsive relay indefinitely.
		if os.clock() - self.lastAck > 4 then
			self:stop()
			self:__state("Debugger relay lease expired")
		end
	end)
	self:__request({ operation = "enable", rootId = self.root, relayId = self.relayId })
		:andThen(function()
			if not self.enabled or generation ~= self.generation then
				return
			end
			self:__schedule(generation)
		end)
		:catch(function(err)
			if not self.enabled or generation ~= self.generation then
				return
			end
			self:stop()
			self:__state("Debugger consent relay rejected: " .. tostring(err):sub(1, 256))
		end)
end

function DebuggerPlans:stop()
	if not self.enabled then
		return
	end
	self.enabled = false
	self.generation += 1
	if self.timer then
		task.cancel(self.timer)
		self.timer = nil
	end
	if self.request then
		local request = self.request
		-- Stop may be invoked by this request's own completion callback.
		task.defer(function()
			request:cancel()
		end)
		self.request = nil
	end
	if self.modeConnection then
		self.modeConnection:Disconnect()
		self.modeConnection = nil
	end
	self:__clear()
	local api = self.session.__apiContext
	if api.__connected then
		api:__debuggerRelay({ operation = "disable", relayId = self.relayId }):catch(function() end)
	end
	self:__state("Disabled")
end

return DebuggerPlans
