-- Clients on the no-mic pulse socket see every object but sources and the
-- monitor ports of sinks, so an app the user took the microphone from can
-- still play, but hears neither the mic nor what the device plays
-- (docs/isolation.md).
log = Log.open_topic ("s-gh0st-nomic")

local ACCESS = "gh0st-nomic"

local function restricted (props)
  return props["pipewire.access"] == ACCESS or props["pipewire.client.access"] == ACCESS
end

local function hidden_node (node)
  local c = node.properties["media.class"] or ""
  return c:find ("^Audio/Source") ~= nil or c:find ("^Stream/Input") ~= nil
      or c:find ("^Video/Source") ~= nil
end

local function hidden_port (port)
  return port.properties["port.monitor"] == "true"
end

clients_om = ObjectManager { Interest { type = "client" } }
nodes_om = ObjectManager { Interest { type = "node" } }
ports_om = ObjectManager { Interest { type = "port" } }

local function apply (client)
  local perms = { ["any"] = "all" }
  for node in nodes_om:iterate () do
    if hidden_node (node) then
      perms[node["bound-id"]] = "-"
    end
  end
  for port in ports_om:iterate () do
    if hidden_port (port) then
      perms[port["bound-id"]] = "-"
    end
  end
  client:update_permissions (perms)
  log:info (client, "no mic for client " .. client["bound-id"])
end

local function hide (id)
  for client in clients_om:iterate () do
    if restricted (client.properties) then
      client:update_permissions { [id] = "-" }
    end
  end
end

clients_om:connect ("object-added", function (_, client)
  if restricted (client.properties) then
    apply (client)
  end
end)

nodes_om:connect ("object-added", function (_, node)
  if hidden_node (node) then
    hide (node["bound-id"])
  end
end)

ports_om:connect ("object-added", function (_, port)
  if hidden_port (port) then
    hide (port["bound-id"])
  end
end)

nodes_om:activate ()
ports_om:activate ()
clients_om:activate ()
