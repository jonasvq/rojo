local Greeting = {}

function Greeting.message(name: string): string
	assert(type(name) == "string", "name must be a string")
	return ("Hello, %s!"):format(name)
end

return table.freeze(Greeting)
