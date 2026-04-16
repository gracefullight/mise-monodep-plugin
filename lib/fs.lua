local M = {}

local function quote(path)
    return '"' .. path:gsub('"', '\\"') .. '"'
end

function M.ensure_dir(path)
    local command = "mkdir -p " .. quote(path)
    local result = os.execute(command)
    if result ~= 0 and result ~= true then
        error("Failed to create directory: " .. path)
    end
end

function M.read_file(path)
    local file = io.open(path, "rb")
    if file == nil then
        error("Failed to open file: " .. path)
    end
    local content = file:read("*a")
    file:close()
    return content
end

function M.write_file(path, content)
    local parent = path:match("(.+)/[^/]+$")
    if parent ~= nil then
        M.ensure_dir(parent)
    end
    local file = io.open(path, "wb")
    if file == nil then
        error("Failed to write file: " .. path)
    end
    file:write(content)
    file:close()
end

function M.make_executable(path)
    local command = "chmod +x " .. quote(path)
    local result = os.execute(command)
    if result ~= 0 and result ~= true then
        error("Failed to chmod file: " .. path)
    end
end

function M.exists(path)
    local file = io.open(path, "r")
    if file ~= nil then
        file:close()
        return true
    end
    return false
end

function M.remove_path(path)
    local command = "rm -rf " .. quote(path)
    local result = os.execute(command)
    if result ~= 0 and result ~= true then
        error("Failed to remove path: " .. path)
    end
end

function M.copy_dir(src, dst)
    M.remove_path(dst)
    M.ensure_dir(dst)
    local command = "cp -R " .. quote(src .. "/.") .. " " .. quote(dst)
    local result = os.execute(command)
    if result ~= 0 and result ~= true then
        error("Failed to copy directory from " .. src .. " to " .. dst)
    end
end

return M
