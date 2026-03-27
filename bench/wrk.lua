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

local function check_recursive(obj, structure)
  if type(structure) ~= "table" then
    return type(obj) == type(structure)
  end

  if type(obj) ~= "table" then
    return false
  end

  if is_array(structure) then
    if obj[1] == nil then
      return false
    end

    return check_recursive(obj[1], structure[1])
  end

  for key, expected in pairs(structure) do
    if obj[key] == nil then
      return false
    end

    if not check_recursive(obj[key], expected) then
      return false
    end
  end

  return true
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

  if decoded ~= nil then
    return check_recursive(decoded, expected_structure)
  end

  return body ~= nil
    and find(body, '"data"', 1, true) ~= nil
    and find(body, '"users"', 1, true) ~= nil
    and find(body, '"topProducts"', 1, true) ~= nil
    and find(body, '"reviews"', 1, true) ~= nil
    and find(body, '"product"', 1, true) ~= nil
    and find(body, '"author"', 1, true) ~= nil
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
