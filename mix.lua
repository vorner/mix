register_config(function (mailbox)
    local name = mailbox:name()
    local path = mailbox:path()
    name = name:gsub(".gz$", " (Archive)")
    name = name:gsub("_", " ")
    mailbox:set_name(name)
end)
