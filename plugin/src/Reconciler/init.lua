--[[
	This module defines the meat of the Rojo plugin and how it manages tracking
	and mutating the Roblox DOM.
]]

local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin

local Timer = require(Plugin.Timer)

local applyPatch = require(script.applyPatch)
local hydrate = require(script.hydrate)
local diff = require(script.diff)

local Reconciler = {}
Reconciler.__index = Reconciler

-- Retain identity only for the most recently disconnected session. Weak values
-- must not keep removed Studio instances alive, and another server must not
-- inherit ownership merely because its instance names happen to match.
local previousSession

function Reconciler.new(instanceMap)
	local self = {
		-- Tracks all of the instances known by the reconciler by ID.
		__instanceMap = instanceMap,
	}

	return setmetatable(self, Reconciler)
end

function Reconciler:applyPatch(patch, adoptMatchingInstances)
	Timer.start("Reconciler:applyPatch")

	local unappliedPatch, conflict = applyPatch(self.__instanceMap, patch, adoptMatchingInstances)

	Timer.stop()
	return unappliedPatch, conflict
end

function Reconciler:rememberSession(sessionId, rootInstance)
	local instances = setmetatable({}, { __mode = "v" })
	for id, instance in self.__instanceMap.fromIds do
		instances[id] = instance
	end
	previousSession = {
		id = sessionId,
		root = rootInstance,
		rootId = self.__instanceMap.fromInstances[rootInstance],
		instances = instances,
	}
end

function Reconciler:hydrate(virtualInstances, rootId, rootInstance, sessionId)
	Timer.start("Reconciler:hydrate")
	local remembered
	if
		previousSession
		and sessionId ~= nil
		and previousSession.id == sessionId
		and previousSession.root == rootInstance
		and previousSession.rootId == rootId
	then
		remembered = previousSession.instances
	end
	local result = hydrate(self.__instanceMap, virtualInstances, rootId, rootInstance, remembered)
	Timer.stop()

	return result
end

function Reconciler:diff(virtualInstances, rootId)
	Timer.start("Reconciler:diff")
	local success, result = diff(self.__instanceMap, virtualInstances, rootId)
	Timer.stop()

	return success, result
end

return Reconciler
