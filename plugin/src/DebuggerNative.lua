-- Native calls here never yield. Only scalar snapshots leave a stop generation.
local DebuggerNative = {}
DebuggerNative.__index = DebuggerNative

local actions = {
	Into = Enum.DebuggerResumeType.StepInto,
	Over = Enum.DebuggerResumeType.StepOver,
	Out = Enum.DebuggerResumeType.StepOut,
	Resume = Enum.DebuggerResumeType.Resume,
}
local comparisons = {
	Eq = function(a, b)
		return a == b
	end,
	Ne = function(a, b)
		return a ~= b
	end,
	Lt = function(a, b)
		return a < b
	end,
	Le = function(a, b)
		return a <= b
	end,
	Gt = function(a, b)
		return a > b
	end,
	Ge = function(a, b)
		return a >= b
	end,
}
local arithmetic = { Add = "+", Subtract = "-", Multiply = "*", Divide = "/" }
local keywords = {}
for word in
	("and break continue do else elseif end false for function if in local nil not or repeat return then true until while"):gmatch(
		"%S+"
	)
do
	keywords[word] = true
end
local function identifier(value)
	return type(value) == "string"
		and #value <= 64
		and value:match("^[A-Za-z_][A-Za-z0-9_]*$") ~= nil
		and not keywords[value]
end
local function finite(value)
	return type(value) == "number" and value == value and math.abs(value) < math.huge
end
local function text(value, limit)
	local kind = type(value)
	local result = if kind == "string"
		then value
		elseif kind == "number" or kind == "boolean" then tostring(value)
		else "<" .. kind .. ">"
	if not utf8.len(result) then
		return "<invalid UTF-8>"
	end
	if #result <= limit then
		return result
	end
	local boundary = utf8.offset(result, 0, limit + 1)
	return result:sub(1, boundary - 1)
end

function DebuggerNative.validate(plan)
	if
		type(plan) ~= "table"
		or type(plan.points) ~= "table"
		or #plan.points < 1
		or #plan.points > 8
		or type(plan.actions) ~= "table"
		or #plan.actions < 1
		or #plan.actions > 16
	then
		return false
	end
	if
		not finite(plan.maxHits)
		or plan.maxHits % 1 ~= 0
		or plan.maxHits < 1
		or plan.maxHits > 64
		or not finite(plan.tableDepth)
		or plan.tableDepth % 1 ~= 0
		or plan.tableDepth < 0
		or plan.tableDepth > 2
	then
		return false
	end
	local seen = {}
	for _, line in plan.points do
		if not finite(line) or line % 1 ~= 0 or line < 1 or line > 1048577 or seen[line] then
			return false
		end
		seen[line] = true
	end
	for _, action in plan.actions do
		if not actions[action] then
			return false
		end
	end
	if
		plan.filter
		and (
			not identifier(plan.filter.identifier)
			or not comparisons[plan.filter.operator]
			or not finite(plan.filter.value)
		)
	then
		return false
	end
	if type(plan.evaluations) ~= "table" or #plan.evaluations > 4 then
		return false
	end
	for _, eval in plan.evaluations do
		if not identifier(eval.identifier) then
			return false
		end
		if
			eval.arithmetic
			and (
				not arithmetic[eval.arithmetic.operator]
				or not finite(eval.arithmetic.operand)
				or eval.arithmetic.operator == "Divide" and eval.arithmetic.operand == 0
			)
		then
			return false
		end
	end
	return true
end

function DebuggerNative.new(service, emit, invalidated)
	return setmetatable({
		service = service,
		emit = emit,
		invalidated = invalidated,
		entries = {},
		pending = {},
		installed = false,
		diagnostics = { stops = 0, owned = 0, foreign = 0, filtered = 0 },
	}, DebuggerNative)
end

function DebuggerNative:remove(id)
	local entry = self.entries[id]
	if not entry then
		return
	end
	self.entries[id] = nil
	for _, connection in entry.connections do
		connection:Disconnect()
	end
	for line in entry.lines do
		pcall(self.service.RemoveBreakpoint, self.service, entry.instance, line)
	end
	for thread, owner in self.pending do
		if owner.entry == entry then
			self.pending[thread] = nil
		end
	end
end

function DebuggerNative:clear()
	local ids = {}
	for id in self.entries do
		table.insert(ids, id)
	end
	for _, id in ids do
		self:remove(id)
	end
	table.clear(self.pending)
end

function DebuggerNative:stop()
	self:clear()
	if self.installed then
		self.installed = false
		-- Write-only callback: exclusive ownership was explicitly accepted.
		pcall(function()
			self.service.OnStopped = nil
		end)
	end
end

function DebuggerNative:install()
	if self.installed then
		return
	end
	self.service.OnStopped = function(payload)
		local ok, action, thread = pcall(self.onStopped, self, payload)
		if not ok then
			self.diagnostics.error = text(action, 512)
			self:clear()
			return Enum.DebuggerResumeType.Resume
		end
		if action == Enum.DebuggerResumeType.Resume then
			return action
		end
		return action, thread
	end
	self.installed = true
end

function DebuggerNative:arm(desired, instance)
	local id = desired.source.instanceId
	assert(self.installed and DebuggerNative.validate(desired.plan), "Invalid or unconsented debugger plan")
	assert(
		instance.ClassName == desired.source.className
			and instance.Source == desired.source.source
			and instance.Parent ~= nil,
		"Managed source changed"
	)
	self:remove(id)
	local entry = {
		desired = desired,
		instance = instance,
		path = instance:GetFullName(),
		lines = {},
		connections = {},
		hits = 0,
	}
	self.entries[id] = entry
	local function invalidate()
		self:remove(id)
		if self.invalidated then
			self.invalidated(id)
		end
	end
	table.insert(entry.connections, instance:GetPropertyChangedSignal("Source"):Connect(invalidate))
	table.insert(entry.connections, instance.AncestryChanged:Connect(invalidate))
	table.insert(entry.connections, instance:GetPropertyChangedSignal("Name"):Connect(invalidate))
	local ok, err = pcall(function()
		for _, line in desired.plan.points do
			local point = self.service:AddBreakpoint(instance, { Line = line })
			-- Even a nonverified result may own a requested native line.
			entry.lines[point.Line or line] = line
			assert(point.Verified, "Native breakpoint is not verified")
		end
	end)
	if not ok then
		self:remove(id)
		error(err, 0)
	end
	return entry
end

local numericTypes = { double = true, float = true, number = true, int = true, integer = true }
local function numeric(values, name)
	local found
	for _, variable in values do
		if variable.Scope ~= Enum.ScriptVariableScope.Global and variable.Name == name then
			if found then
				return nil, "ambiguous"
			end
			found = variable
		end
	end
	if not found then
		return nil, "missing or optimized"
	end
	if not numericTypes[found.Type] then
		return nil, "non-numeric"
	end
	local number = if type(found.Value) == "number"
		then found.Value
		elseif type(found.Value) == "string" then tonumber(found.Value)
		else nil
	if not finite(number) then
		return nil, "non-finite or unavailable"
	end
	return number
end

function DebuggerNative:onStopped(payload)
	self.diagnostics.stops += 1
	if not self.installed or type(payload) ~= "table" or type(payload.ThreadIds) ~= "table" then
		return Enum.DebuggerResumeType.Resume
	end
	local thread, trace, entry, stoppedPoint
	for i = 1, math.min(#payload.ThreadIds, 64) do
		local candidate = payload.ThreadIds[i]
		local ok, stack = pcall(self.service.GetStackTrace, self.service, candidate)
		if ok and stack.Frames and stack.Frames[1] then
			local frame = stack.Frames[1]
			self.diagnostics.path = text(frame.ScriptPath, 512)
			self.diagnostics.line = frame.Line
			local pending = self.pending[candidate]
			local owner = pending and pending.entry
			local point = pending and pending.point
			if not owner and payload.Reason == Enum.ScriptStoppedReason.Breakpoint then
				for _, active in self.entries do
					if active.path == frame.ScriptPath and active.lines[frame.Line] then
						owner = active
						point = frame.Line
						break
					end
				end
				-- Measured combined-Run call-site stops report the following
				-- instruction's line (verified point4, frame5). Keep the installed
				-- line distinct from the execution line; do not apply to exceptions.
				if not owner and payload.Reason == Enum.ScriptStoppedReason.Breakpoint then
					for _, active in self.entries do
						if active.path == frame.ScriptPath and active.lines[frame.Line - 1] then
							owner = active
							point = frame.Line - 1
							break
						end
					end
				end
			end
			if owner then
				thread = candidate
				trace = stack
				entry = owner
				stoppedPoint = point
				break
			end
		end
	end
	-- Exclusive mode deliberately resumes foreign/manual stops.
	if not entry then
		self.diagnostics.foreign += 1
		return Enum.DebuggerResumeType.Resume
	end
	self.diagnostics.owned += 1
	self.pending[thread] = nil
	local desired = entry.desired
	if
		entry.instance.Parent == nil
		or entry.instance.Source ~= desired.source.source
		or entry.instance.ClassName ~= desired.source.className
	then
		self:remove(desired.source.instanceId)
		return Enum.DebuggerResumeType.Resume
	end
	entry.hits += 1
	local plan = desired.plan
	local action = plan.actions[entry.hits] or "Resume"
	if entry.hits >= plan.maxHits then
		action = "Resume"
	end
	local first = trace.Frames[1]
	local report = {
		hit = entry.hits,
		requestedLine = entry.lines[stoppedPoint],
		verifiedLine = stoppedPoint,
		action = action,
		frames = {},
		evaluations = {},
		overflow = 0,
	}
	local rootsOk, roots = pcall(self.service.GetRootVariables, self.service, first.Id)
	if not rootsOk then
		roots = {}
		report.error = "Native root variables unavailable"
	end
	local capture = true
	if plan.filter then
		local value, err = numeric(roots, plan.filter.identifier)
		if err then
			report.error = "Filter: " .. err
			action = "Resume"
		else
			capture = comparisons[plan.filter.operator](value, plan.filter.value)
		end
	end
	if capture then
		-- Six bytes per input byte covers worst-case JSON escaping. Native
		-- enumeration itself cannot be interrupted; the output is bounded here.
		local remaining = 58000
		local function scalar(variable, depth)
			local row = {
				name = text(variable.Name, 512),
				kind = text(variable.Type, 64),
				value = text(variable.Value, 512),
				children = {},
			}
			local charge = 128 + 6 * (#row.name + #row.kind + #row.value)
			if remaining < charge then
				return nil
			end
			remaining -= charge
			if
				depth < plan.tableDepth
				and type(variable.VariablesReference) == "number"
				and variable.VariablesReference > 0
				and (variable.Type == "LazyTable" or variable.Type == "table")
			then
				local ok, children = pcall(self.service.GetVariables, self.service, variable.VariablesReference)
				if ok then
					for j = 1, math.min(#children, 32) do
						local child = scalar(children[j], depth + 1)
						if child then
							table.insert(row.children, child)
						end
					end
				end
			end
			return row
		end
		for i = 1, math.min(#trace.Frames, 12) do
			local frame = trace.Frames[i]
			local row = { name = text(frame.ScriptPath, 512), line = math.max(0, frame.Line or 0), values = {} }
			if remaining < 6 * #row.name + 128 then
				break
			end
			remaining -= 6 * #row.name + 128
			local ok, values = rootsOk, roots
			if i ~= 1 then
				ok, values = pcall(self.service.GetRootVariables, self.service, frame.Id)
			end
			if ok then
				local count = 0
				for _, variable in values do
					if variable.Scope ~= Enum.ScriptVariableScope.Global then
						count += 1
						if count > 32 then
							break
						end
						local value = scalar(variable, 0)
						if value then
							table.insert(row.values, value)
						end
					end
				end
			end
			table.insert(report.frames, row)
		end
		for _, eval in plan.evaluations do
			local _, err = numeric(roots, eval.identifier)
			local expression = eval.identifier
			if eval.arithmetic then
				expression ..= " " .. arithmetic[eval.arithmetic.operator] .. " (" .. string.format(
					"%.17g",
					eval.arithmetic.operand
				) .. ")"
			end
			local row = { name = expression, kind = "error", value = err or "Native evaluation failed", children = {} }
			if not err then
				local ok, result = pcall(self.service.Evaluate, self.service, expression, first.Id)
				if ok and not result.Error and numericTypes[result.Type] and finite(tonumber(result.Result)) then
					row.kind = "number"
					row.value = text(result.Result, 512)
				end
			end
			local charge = 128 + 6 * (#row.name + #row.kind + #row.value)
			if remaining < charge then
				report.error = "Evaluation output omitted at report budget"
				break
			end
			remaining -= charge
			table.insert(report.evaluations, row)
		end
		report.action = action
		self.emit(desired, report)
	else
		self.diagnostics.filtered += 1
	end
	if entry.hits >= plan.maxHits then
		self:remove(desired.source.instanceId)
		action = "Resume"
	end
	if action == "Resume" then
		return Enum.DebuggerResumeType.Resume
	end
	self.pending[thread] = { entry = entry, point = stoppedPoint }
	return actions[action], thread
end

return DebuggerNative
