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

local function is_array(value)
  return type(value) == "table" and value[1] ~= nil
end

local function sorted_keys(value)
  local keys = {}
  for key, _ in pairs(value) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

local function canonical_json_encode(value)
  local value_type = type(value)

  if value_type == "table" then
    local parts = {}

    if is_array(value) then
      for i = 1, #value do
        parts[#parts + 1] = canonical_json_encode(value[i])
      end
      return "[" .. table.concat(parts, ",") .. "]"
    end

    local keys = sorted_keys(value)
    for _, key in ipairs(keys) do
      local encoded_key = cjson and cjson.encode and cjson.encode(key) or ('"' .. escape_json(key) .. '"')
      parts[#parts + 1] = encoded_key .. ":" .. canonical_json_encode(value[key])
    end
    return "{" .. table.concat(parts, ",") .. "}"
  end

  if cjson and cjson.encode then
    return cjson.encode(value)
  end

  if value_type == "string" then
    return '"' .. escape_json(value) .. '"'
  end

  if value_type == "number" then
    return tostring(value)
  end

  if value_type == "boolean" then
    return value and "true" or "false"
  end

  return "null"
end

local function hash_string(value)
  local hash = 5381
  local max_u32 = 4294967296

  for i = 1, #value do
    hash = ((hash * 33) + value:byte(i)) % max_u32
  end

  return string.format("%08x", hash)
end

local function normalize_with_template(value, template)
  if type(template) ~= "table" then
    if type(value) ~= type(template) then
      return nil
    end

    return type(template)
  end

  if type(value) ~= "table" then
    return nil
  end

  if is_array(template) then
    if value[1] == nil then
      return nil
    end

    local first = normalize_with_template(value[1], template[1])
    if first == nil then
      return nil
    end

    return { first }
  end

  local normalized = {}
  for key, expected in pairs(template) do
    if value[key] == nil then
      return nil
    end

    local normalized_child = normalize_with_template(value[key], expected)
    if normalized_child == nil then
      return nil
    end

    normalized[key] = normalized_child
  end

  return normalized
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

local expected_structure = {
  data = {
    users = {
      {
        id = "1",
        username = "urigo",
        name = "Uri Goldshtein",
        reviews = {
          {
            id = "",
            body = "",
            product = {
              inStock = true,
              name = "Table",
              price = 899,
              shippingEstimate = 50,
              upc = "1",
              weight = 100,
              reviews = {
                {
                  id = "",
                  body = "",
                  author = {
                    id = "1",
                    username = "urigo",
                    name = "Uri Goldshtein",
                    reviews = {
                      {
                        id = "",
                        body = "",
                        product = {
                          inStock = true,
                          name = "Table",
                          price = 899,
                          shippingEstimate = 50,
                          upc = "1",
                          weight = 100,
                        },
                      },
                    },
                  },
                },
              },
            },
          },
        },
      },
    },
    topProducts = {
      {
        inStock = true,
        name = "Table",
        price = 899,
        shippingEstimate = 50,
        upc = "1",
        weight = 100,
        reviews = {
          {
            id = "",
            body = "",
            author = {
              id = "1",
              username = "urigo",
              name = "Uri Goldshtein",
              reviews = {
                {
                  id = "",
                  body = "",
                  product = {
                    inStock = true,
                    name = "Table",
                    price = 899,
                    shippingEstimate = 50,
                    upc = "1",
                    weight = 100,
                  },
                },
              },
            },
          },
        },
      },
    },
  },
}

local expected_structure_hash = hash_string(
  canonical_json_encode(normalize_with_template(expected_structure, expected_structure))
)

local function check_response_structure(body)
  local decoded = nil
  if cjson_safe ~= nil then
    decoded = cjson_safe.decode(body)
  elseif cjson ~= nil then
    local ok_decode, result = pcall(cjson.decode, body)
    if ok_decode then
      decoded = result
    end
  end

  if decoded == nil then
    return false
  end

  local normalized_response = normalize_with_template(decoded, expected_structure)
  if normalized_response == nil then
    return false
  end

  local response_hash = hash_string(canonical_json_encode(normalized_response))
  return response_hash == expected_structure_hash
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
