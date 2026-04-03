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

local cjson_safe = nil
local cjson = nil

local function build_graphql_request_body(query)
  if cjson_safe ~= nil and cjson_safe.encode ~= nil then
    local encoded = cjson_safe.encode({ query = query })
    if encoded ~= nil then
      return encoded
    end
  end

  if cjson ~= nil and cjson.encode ~= nil then
    local ok_encode, encoded = pcall(cjson.encode, { query = query })
    if ok_encode and encoded ~= nil then
      return encoded
    end
  end

  local escaped_query = escape_json(query)
  return "{\"query\":\"" .. escaped_query .. "\"}"
end

local function hash_string(value)
  local hash = 5381
  local max_u32 = 4294967296

  for i = 1, #value do
    hash = ((hash * 33) + value:byte(i)) % max_u32
  end

  return string.format("%08x", hash)
end
local ok_safe, cjson_safe_module = pcall(require, "cjson.safe")
if ok_safe then
  cjson_safe = cjson_safe_module
end

local ok_cjson, cjson_module = pcall(require, "cjson")
if ok_cjson then
  cjson = cjson_module
end

local status_failures = 0
local graphql_error_responses = 0
local response_structure_failures = 0
local checked_response_structure = false
local sample_status_failure = nil
local sample_graphql_error = nil
local sample_structure_error = nil
local find = string.find

local expected_response_file = os.getenv("BENCH_EXPECTED_RESPONSE_FILE")
local expected_response

if expected_response_file ~= nil and expected_response_file ~= "" then
  expected_response = read_file(expected_response_file)
else
  expected_response = read_file_if_exists("expected_response.json") or read_file("bench/expected_response.json")
end

local expected_response_hash = hash_string(expected_response)

local function check_response_structure(body)
  if body == nil then
    return false
  end

  local response_hash = hash_string(body)
  return response_hash == expected_response_hash
end

local operation_file = os.getenv("BENCH_OPERATION_FILE")
local query

if operation_file ~= nil and operation_file ~= "" then
  query = read_file(operation_file)
else
  query = read_file_if_exists("operation.graphql") or read_file("bench/operation.graphql")
end

wrk.method = "POST"
wrk.headers["Content-Type"] = "application/json"
wrk.body = build_graphql_request_body(query)

response = function(status, headers, body)
  if status ~= 200 then
    status_failures = status_failures + 1
    if sample_status_failure == nil then
      sample_status_failure = body
    end
    return
  end

  if body and find(body, '"errors"', 1, true) then
    graphql_error_responses = graphql_error_responses + 1
    if sample_graphql_error == nil then
      sample_graphql_error = body
    end
    return
  end

  if checked_response_structure then
    return
  end

  checked_response_structure = true

  if not check_response_structure(body) then
    response_structure_failures = response_structure_failures + 1
    if sample_structure_error == nil then
      sample_structure_error = body
    end
  end
end

done = function(summary, latency, requests)
  io.write("VALIDATION_STATUS_FAILURES=" .. status_failures .. "\n")
  io.write("VALIDATION_GRAPHQL_ERRORS=" .. graphql_error_responses .. "\n")
  io.write("VALIDATION_RESPONSE_STRUCTURE_FAILURES=" .. response_structure_failures .. "\n")

  if sample_status_failure ~= nil then
    io.write("VALIDATION_STATUS_FAILURE_SAMPLE=" .. sample_status_failure .. "\n")
  end
  if sample_graphql_error ~= nil then
    io.write("VALIDATION_GRAPHQL_ERROR_SAMPLE=" .. sample_graphql_error .. "\n")
  end
  if sample_structure_error ~= nil then
    io.write("VALIDATION_RESPONSE_STRUCTURE_FAILURE_SAMPLE=" .. sample_structure_error .. "\n")
  end
end
