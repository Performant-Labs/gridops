-- Jobs find their runner by name to show where they ran.
CREATE INDEX runners_name_idx ON runners(name, created_at);
