-- Egress IP isolation: allow different API keys to route through different outbound IPs
ALTER TABLE registered_api_keys ADD COLUMN egress_ip TEXT NOT NULL DEFAULT 'default';
