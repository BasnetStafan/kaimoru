-- The free-space section's key catches up with its name:
-- the CMS section is a free space (label フリースペース)
-- whose first content promotes the agent, so its stored
-- key 'agent' named the content, not the section. The key
-- becomes 'free_space' everywhere (code, form fields, row).
UPDATE home_sections SET section_key = 'free_space' WHERE section_key = 'agent';
