local HttpService = game:GetService("HttpService")
local Packages = script.Parent.Parent.Packages
local Promise = require(Packages.Promise)
local Log = require(Packages.Log)
local setProperty = require(script.Parent.Reconciler.setProperty)

local SourceWrites = {}
SourceWrites.__index = SourceWrites
local LIMIT = 64
-- Recovery protection is scoped to live instances in this plugin process. It
-- is not an ownership attribute or a way to adopt objects after a restart.
local unreviewed = setmetatable({}, { __mode = "k" })

local function sourceOf(instance)
	local ok, source = pcall(function()
		return instance.Source
	end)
	return if ok and type(source) == "string" then source else nil
end

local function bounded(value)
	local text = tostring(value)
	local length = math.min(#text, 512)
	while length > 0 and utf8.len(text:sub(1, length)) == nil do
		length -= 1
	end
	return text:sub(1, length)
end

local function propertySource(record)
	local source = record and record.Properties and record.Properties.Source
	return if source and type(source.String) == "string" then source.String else nil
end

function SourceWrites.needsReview()
	for instance in unreviewed do
		if instance:IsDescendantOf(game) then
			return true
		end
	end
	return false
end

function SourceWrites.requireReview(instance)
	-- A replacement container can hold an unconfirmed descendant Source after
	-- Destroy erased the original ancestry. This flag grants no ownership.
	unreviewed[instance] = true
end

function SourceWrites.reviewedIn(instanceMap)
	local reviewed = {}
	for instance in unreviewed do
		if instanceMap.fromInstances[instance] ~= nil then
			table.insert(reviewed, instance)
		end
	end
	return reviewed
end

function SourceWrites.clearReviewed(reviewed)
	for _, instance in reviewed do
		unreviewed[instance] = nil
	end
end

function SourceWrites.new(api, instanceMap)
	return setmetatable({
		api = api,
		map = instanceMap,
		active = true,
		states = {},
		pending = 0,
		observers = {},
		origin = HttpService:GenerateGUID(false),
		jobs = {},
	}, SourceWrites)
end

function SourceWrites:__notify()
	if not self.active then
		return
	end
	for callback in self.observers do
		local ok, err = pcall(callback, self:summary())
		if not ok then
			Log.warn("Source write observer failed: {}", err)
		end
	end
end

function SourceWrites:observe(callback)
	self.observers[callback] = true
	callback(self:summary())
	return function()
		self.observers[callback] = nil
	end
end

function SourceWrites:summary()
	local issues = {}
	for id, state in self.states do
		if state.issue then
			local instance = self.map.fromIds[id]
			table.insert(issues, {
				id = id,
				reason = state.issue,
				path = bounded(if instance then instance:GetFullName() else id),
				sourcePath = state.sourcePath,
				running = state.running == true,
				reviewed = state.reviewed ~= nil,
			})
		end
	end
	table.sort(issues, function(a, b)
		return a.id < b.id
	end)
	return { pending = self.pending, issues = issues }
end

function SourceWrites:__issue(id, reason)
	local state = self.states[id]
	if not state or not self.active then
		return
	end
	local message = bounded(reason)
	if state.issue == message then
		return
	end
	state.issue = message
	state.reviewed = nil
	local instance = self.map.fromIds[id]
	if instance then
		unreviewed[instance] = true
	end
	Log.warn("Source write paused for {}: {}", if instance then instance:GetFullName() else id, state.issue)
	self:__notify()
end

function SourceWrites:seed(instances, bindings)
	for id, record in instances do
		local source = propertySource(record)
		if source then
			local binding = bindings and bindings[id]
			self.states[id] = {
				baseSource = source,
				className = record.ClassName,
				bindingHash = binding and binding.bindingHash,
				sourceHash = binding and binding.sourceHash,
				generation = 0,
			}
		end
	end
end

function SourceWrites:protects(parent)
	if not parent then
		return false
	end
	for id, state in self.states do
		local instance = self.map.fromIds[id]
		if
			instance
			and (state.pendingSource ~= nil or state.running or state.issue)
			and (instance == parent or instance:IsDescendantOf(parent))
		then
			return true
		end
	end
	return false
end

function SourceWrites:writeInitial(patch)
	if next(patch.added) ~= nil or #patch.removed ~= 0 then
		return Promise.reject(
			"Initial reverse sync supports existing Source changes only; additions/removals need a separate ownership decision"
		)
	end
	for _, update in patch.updated do
		if update.changedName or update.changedClassName then
			return Promise.reject("Initial reverse sync cannot change names or classes")
		end
		for property in update.changedProperties do
			if property ~= "Source" then
				return Promise.reject("Initial reverse sync supports Source only; unsupported property: " .. property)
			end
		end
	end
	for _, update in patch.updated do
		local source = update.changedProperties.Source
		if source and type(source.String) == "string" then
			self:edit(update.id, source.String)
		end
	end
	return Promise.new(function(resolve, reject)
		while self.active do
			local summary = self:summary()
			if #summary.issues > 0 then
				reject(summary.issues[1].reason)
				return
			end
			if summary.pending == 0 then
				resolve()
				return
			end
			task.wait(0.05)
		end
		reject("Source receipt connection ended before initial writes were confirmed")
	end)
end

function SourceWrites:__fresh(id, state)
	local expectedInstance = self.map.fromIds[id]
	return self.api:sourceView(id):timeout(8):andThen(function(view)
		if not self.active or self.states[id] ~= state then
			return Promise.reject("Source write connection ended")
		end
		local instance = self.map.fromIds[id]
		if not instance or instance ~= expectedInstance or instance.ClassName ~= view.className then
			return Promise.reject("Source class or identity changed; reconnect and review")
		end
		state.sourcePath = bounded(view.sourcePath or "")
		return view
	end)
end

function SourceWrites:__start(id, state)
	if not self.active or state.running or state.issue or state.pendingSource == nil then
		return
	end
	if self.pending >= LIMIT then
		state.pendingSource = nil
		self:__issue(id, "Source write queue is full; local edits were retained. Review before retrying.")
		return
	end
	if not state.bindingHash then
		self:__issue(id, "The accepted source binding is unavailable; review the file before writing.")
		return
	end
	if state.pendingSource == state.baseSource then
		state.pendingSource = nil
		local instance = self.map.fromIds[id]
		if instance then
			unreviewed[instance] = nil
		end
		return
	end
	state.running = true
	state.sending = state.pendingSource
	self.pending += 1
	local sent = state.sending
	local expectedBase, expectedBinding = state.baseSource, state.bindingHash
	local requestId = HttpService:GenerateGUID(false)
	state.job = Promise.new(function(resolve, reject, onCancel)
		local waiting
		onCancel(function()
			if waiting then
				waiting:cancel()
			end
		end)
		local function await(promise)
			waiting = promise:timeout(8)
			local ok, value = waiting:await()
			waiting = nil
			if not self.active then
				error("Source write connection ended", 0)
			end
			if not ok then
				error(tostring(value), 0)
			end
			return value
		end
		local ok, result = pcall(function()
			local base = await(self:__fresh(id, state))
			if state.issue or base.source ~= expectedBase or base.bindingHash ~= expectedBinding then
				error("File source or binding changed since the accepted base; review before writing", 0)
			end
			local admission = await(self.api:submitSource({
				originId = self.origin,
				requestId = requestId,
				instanceId = id,
				className = base.className,
				sourceHash = base.sourceHash,
				bindingHash = base.bindingHash,
				source = sent,
			}))
			local deadline = os.clock() + 20
			while true do
				local receipt = await(self.api:sourceReceipt(admission.receiptId, self.origin))
				if
					receipt.originId ~= nil and receipt.originId ~= self.origin
					or receipt.requestId ~= nil and receipt.requestId ~= requestId
				then
					error("Source receipt identity changed", 0)
				end
				if receipt.state ~= "pending" then
					return receipt
				end
				if os.clock() > deadline then
					error("Source receipt did not finish; persistence is uncertain. Review before retrying.", 0)
				end
				task.wait(0.1)
			end
		end)
		if ok then
			resolve(result)
		else
			reject(result)
		end
	end)
		:andThen(function(receipt)
			if not self.active then
				return
			end
			if receipt.state ~= "applied" and receipt.state ~= "unchanged" then
				self:__issue(id, receipt.reason or "Source persistence was not confirmed")
				return
			end
			if receipt.bindingHash ~= expectedBinding or type(receipt.sourceHash) ~= "string" then
				self:__issue(id, "Source receipt did not confirm the expected binding")
				return
			end
			if state.issue then
				return
			end
			state.baseSource = sent:gsub("\r\n", "\n")
			state.sourceHash = receipt.sourceHash
			if state.pendingSource == sent then
				state.pendingSource = nil
			end
			if state.pendingSource == nil then
				local instance = self.map.fromIds[id]
				if instance then
					unreviewed[instance] = nil
				end
			end
		end)
		:catch(function(reason)
			if self.active then
				self:__issue(id, "Source persistence is unconfirmed: " .. bounded(reason))
			end
		end)
		:finally(function()
			if not self.active then
				return
			end
			state.running = false
			state.sending = nil
			state.job = nil
			self.pending -= 1
			self:__notify()
			self:__start(id, state)
		end)
	self:__notify()
end

function SourceWrites:edit(id, source)
	if not self.active or type(source) ~= "string" then
		return
	end
	local state = self.states[id]
	if not state then
		state = { baseSource = nil, generation = 0 }
		self.states[id] = state
	end
	state.pendingSource = source
	if state.issue then
		state.reviewed = nil
		state.reviewedStudio = nil
		self:__notify()
	end
	local instance = self.map.fromIds[id]
	if instance then
		unreviewed[instance] = true
	end
	self:__start(id, state)
end

function SourceWrites:__refreshBindings(ids)
	self.bindingQueue = self.bindingQueue or {}
	for _, id in ids do
		self.bindingQueue[id] = true
	end
	self:__pumpBindings()
end

function SourceWrites:__pumpBindings()
	if not self.active or self.bindingReadPending then
		return
	end
	local ids = {}
	for id in self.bindingQueue or {} do
		self.bindingQueue[id] = nil
		if self.states[id] then
			table.insert(ids, id)
		end
		if #ids == LIMIT then
			break
		end
	end
	if #ids == 0 then
		return
	end
	self.bindingReadPending = true
	self:__readBindings(ids)
end

function SourceWrites:__readBindings(ids)
	if #ids == 0 or not self.api.read then
		self.bindingReadPending = false
		return
	end
	-- Bound each read and accept its baseline only before any local edit. This
	-- is used for newly added/changed bindings, never to rebase a dirty source.
	local generations = {}
	for _, id in ids do
		local state = self.states[id]
		generations[id] = state and state.generation
	end
	local job
	job = self.api
		:read(ids)
		:timeout(8)
		:andThen(function(response)
			if not self.active then
				return
			end
			for _, id in ids do
				local state = self.states[id]
				local record = response.instances and response.instances[id]
				local binding = response.writeSources and response.writeSources[id]
				if
					state
					and not state.pendingSource
					and not state.running
					and not state.issue
					and state.generation == generations[id]
					and record
					and binding
					and propertySource(record) == state.baseSource
					and record.ClassName == state.className
				then
					state.bindingHash = binding.bindingHash
					state.sourceHash = binding.sourceHash
				end
			end
		end)
		:catch(function() end)
		:finally(function()
			if job then
				self.jobs[job] = nil
			end
			self.bindingReadPending = false
			self:__pumpBindings()
		end)
	self.jobs[job] = true
end

function SourceWrites:prepare(patch)
	local ids = {}
	local removedSet, retired = {}, {}
	for _, removed in patch.removed or {} do
		removedSet[removed] = true
	end
	if next(removedSet) then
		for id, state in self.states do
			local removed = removedSet[id]
			local ancestor = self.map.fromIds[id]
			while not removed and ancestor do
				removed = removedSet[ancestor] or removedSet[self.map.fromInstances[ancestor]]
				ancestor = ancestor.Parent
			end
			if removed then
				if state.pendingSource ~= nil or state.running or state.issue then
					return false,
						"A managed source with unconfirmed local edits was removed. Local source was retained; review it before reconnecting."
				end
				table.insert(retired, id)
			end
		end
	end
	for _, id in retired do
		self.states[id] = nil
		if self.bindingQueue then
			self.bindingQueue[id] = nil
		end
	end
	for _, update in patch.updated or {} do
		local state = self.states[update.id]
		if state then
			local dirty = state.pendingSource ~= nil or state.running or state.issue
			if dirty and (update.changedClassName or update.changedName or update.changedMetadata) then
				return false,
					"A managed source binding changed while local edits were unconfirmed. Local source was retained; review before reconnecting."
			end
			local incoming = update.changedProperties and update.changedProperties.Source
			if incoming then
				local source = incoming.String
				if type(source) ~= "string" then
					return false,
						"A managed source was reset while receipt tracking was active; review before reconnecting."
				end
				if
					dirty
					and source ~= state.sending
					and source ~= state.pendingSource
					and source ~= state.baseSource
				then
					self:__issue(
						update.id,
						"A different file source arrived while Studio had local edits; review both versions."
					)
				end
				if dirty then
					local instance = self.map.fromIds[update.id]
					if state.issue or instance and sourceOf(instance) ~= source then
						update.changedProperties.Source = nil
					end
				end
				if not state.issue then
					state.baseSource = source
					state.sourceHash = nil
				end
			end
			if not dirty and (update.changedClassName or update.changedName or update.changedMetadata) then
				state.className = update.changedClassName or state.className
				state.bindingHash = nil
				state.generation += 1
				table.insert(ids, update.id)
			end
		end
	end
	for id, record in patch.added or {} do
		local source = propertySource(record)
		if source then
			self.states[id] = { baseSource = source, className = record.ClassName, generation = 0 }
			table.insert(ids, id)
		end
	end
	for start = 1, #ids, LIMIT do
		local batch = {}
		for index = start, math.min(start + LIMIT - 1, #ids) do
			table.insert(batch, ids[index])
		end
		self:__refreshBindings(batch)
	end
	self:__notify()
	return true
end

function SourceWrites:protectedIds()
	local protected = {}
	for id, state in self.states do
		if state.pendingSource ~= nil or state.running or state.issue then
			protected[id] = true
		end
	end
	return protected
end

function SourceWrites:review(id)
	local state = self.states[id]
	if not state or state.running then
		return Promise.reject("Wait for the pending receipt before reviewing")
	end
	return self:__fresh(id, state):andThen(function(view)
		if not self.active then
			return Promise.reject("Source write connection ended")
		end
		state.reviewed = view
		state.reviewedStudio = sourceOf(self.map.fromIds[id]) or ""
		self:__notify()
		return { studio = state.reviewedStudio, file = view.source }
	end)
end

function SourceWrites:resolve(id, choice)
	local state = self.states[id]
	if not state or state.running then
		return Promise.reject("Wait for the pending receipt before resolving")
	end
	if choice ~= "file" and choice ~= "studio" then
		return Promise.reject("Unknown source resolution")
	end
	if choice == "studio" and not state.reviewed then
		return Promise.reject("Review both sources before keeping Studio changes")
	end
	local instance = self.map.fromIds[id]
	local localSource = instance and sourceOf(instance)
	if choice == "studio" and localSource ~= state.reviewedStudio then
		self:__issue(id, "Studio source changed after review; review both sources again")
		return Promise.reject(state.issue)
	end
	return self:__fresh(id, state):andThen(function(view)
		if not self.active or self.map.fromIds[id] ~= instance then
			return Promise.reject("Source write connection or identity changed")
		end
		if sourceOf(instance) ~= localSource then
			return Promise.reject("Studio source changed during review; review again")
		end
		if
			choice == "studio"
			and (state.reviewed.sourceHash ~= view.sourceHash or state.reviewed.bindingHash ~= view.bindingHash)
		then
			self:__issue(id, "File source changed again; review the new file before keeping Studio changes")
			return Promise.reject(state.issue)
		end
		if choice == "file" then
			self.map:pauseInstance(instance)
			local ok = setProperty(instance, "Source", view.source)
			if not ok then
				return Promise.reject("Could not restore file Source in Studio; local source was retained")
			end
			state.pendingSource = nil
			unreviewed[instance] = nil
		else
			state.pendingSource = localSource
		end
		state.baseSource, state.sourceHash, state.bindingHash = view.source, view.sourceHash, view.bindingHash
		state.issue, state.reviewed = nil, nil
		if choice == "file" and self.map.unpauseInstance then
			self.map:unpauseInstance(instance)
		end
		self:__notify()
		self:__start(id, state)
	end)
end

function SourceWrites:stop()
	if not self.active then
		return
	end
	self.active = false
	for id, state in self.states do
		if state.pendingSource ~= nil or state.running or state.issue then
			local instance = self.map.fromIds[id]
			if instance then
				unreviewed[instance] = true
			end
		end
		if state.job then
			local job = state.job
			task.defer(function()
				job:cancel()
			end)
		end
	end
	for job in self.jobs do
		task.defer(function()
			job:cancel()
		end)
	end
	table.clear(self.jobs)
	table.clear(self.observers)
end

return SourceWrites
