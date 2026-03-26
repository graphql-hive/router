local function read_file(path)
  local file = io.open(path, "r")
  if file == nil then
    error("Unable to open file: " .. path)
  end

  local contents = file:read("*a")
  file:close()
  return contents
end

local function read_file_if_exists(path)
  local file = io.open(path, "r")
  if file == nil then
    return nil
  end

  local contents = file:read("*a")
  file:close()
  return contents
end

local function escape_json(value)
  value = value:gsub("\\", "\\\\")
  value = value:gsub('"', '\\"')
  value = value:gsub("\n", "\\n")
  value = value:gsub("\r", "\\r")
  value = value:gsub("\t", "\\t")
  return value
end

local status_failures = 0
local graphql_error_responses = 0
local find = string.find

local operation_file = os.getenv("BENCH_OPERATION_FILE")
local query

if operation_file ~= nil and operation_file ~= "" then
  query = read_file(operation_file)
else
  query = read_file_if_exists("operation.graphql") or read_file("bench/operation.graphql")
end

local escaped_query = escape_json(query)

wrk.method = "POST"
wrk.headers["Content-Type"] = "application/json"
wrk.body = "{\"query\":\"" .. escaped_query .. "\"}"

response = function(status, headers, body)
  if status ~= 200 then
    status_failures = status_failures + 1
    return
  end

  if body and find(body, '"errors"', 1, true) then
    graphql_error_responses = graphql_error_responses + 1
  end
end

done = function(summary, latency, requests)
  local p95_us = latency:percentile(95.0)
  local p99_us = latency:percentile(99.0)

  io.write("VALIDATION_STATUS_FAILURES=" .. status_failures .. "\n")
  io.write("VALIDATION_GRAPHQL_ERRORS=" .. graphql_error_responses .. "\n")
end
