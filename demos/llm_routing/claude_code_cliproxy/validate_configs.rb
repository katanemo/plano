#!/usr/bin/env ruby
# frozen_string_literal: true

require "psych"
require "set"

EXPECTED_MODELS = Set[
  "anthropic/gpt-5.6-sol",
  "anthropic/gpt-5.6-terra",
  "anthropic/gpt-5.6-luna"
].freeze

EXPECTED_ROUTES = {
  "deep-technical-reasoning" => {
    "description" => "Diagnosis-only reasoning with no code or file writing, modification, refactoring, review, or testing; includes root-cause, deadlocks and concurrency, architecture, security, and incident analysis; excludes trivial formatting, renaming, factual lookup, and simple summary.",
    "model" => "anthropic/gpt-5.6-sol"
  },
  "software-implementation" => {
    "description" => "The requested deliverable includes nontrivial writing, modifying, refactoring, reviewing, or testing of code or files; excludes diagnosis-only reasoning and trivial formatting or renaming.",
    "model" => "anthropic/gpt-5.6-terra"
  },
  "quick-developer-utility" => {
    "description" => "Only trivial formatting or renaming, short factual lookup, or simple summary; excludes debugging, root-cause or incident analysis, architecture, security, code review, testing, and code implementation.",
    "model" => "anthropic/gpt-5.6-luna"
  }
}.freeze

EXPECTED_ALIASES = {
  "claude-fable-5" => "anthropic/gpt-5.6-sol",
  "claude-opus-4-8" => "anthropic/gpt-5.6-terra",
  "claude-sonnet-5" => "anthropic/gpt-5.6-luna",
  "claude-haiku-4-5" => "anthropic/gpt-5.6-luna"
}.freeze


def fail_validation(message)
  warn "validate_configs: #{message}"
  exit 1
end


def load_yaml(path, label)
  parsed = Psych.safe_load_file(path, permitted_classes: [], permitted_symbols: [], aliases: false)
  fail_validation("#{label} root must be a mapping") unless parsed.is_a?(Hash)

  parsed
rescue Errno::ENOENT
  fail_validation("#{label} file not found")
rescue Psych::Exception
  fail_validation("#{label} must be valid safe YAML without aliases or object tags")
end


def validate_plano(config)
  fail_validation("Plano version must be v0.4.0") unless config["version"] == "v0.4.0"

  listeners = config["listeners"]
  fail_validation("Plano must define exactly one listener") unless listeners.is_a?(Array) && listeners.length == 1

  listener = listeners.first
  fail_validation("listeners[0] must be a mapping") unless listener.is_a?(Hash)
  fail_validation("listener type must be model") unless listener["type"] == "model"
  fail_validation("listener address must be 127.0.0.1") unless listener["address"] == "127.0.0.1"
  fail_validation("listener port must be 12000") unless listener["port"] == 12_000

  providers = config["model_providers"]
  fail_validation("Plano must define exactly three model providers") unless providers.is_a?(Array) && providers.length == 3
  fail_validation("model_providers entries must be mappings") unless providers.all? { |provider| provider.is_a?(Hash) }

  provider_models = providers.map { |provider| provider["model"] }
  if provider_models.any? { |model| model.to_s.include?("[1m]") }
    fail_validation("Plano model IDs must not contain [1m]")
  end
  fail_validation("Plano model provider IDs do not match the required tiers") unless provider_models.to_set == EXPECTED_MODELS

  providers.each_with_index do |provider, index|
    fail_validation("model_providers[#{index}].base_url must target local CLIProxyAPI") unless provider["base_url"] == "http://127.0.0.1:8317"
    fail_validation("model_providers[#{index}].access_key must reference CLIPROXY_LOCAL_API_KEY") unless provider["access_key"] == "$CLIPROXY_LOCAL_API_KEY"
  end

  defaults = providers.select { |provider| provider["default"] == true }
  unless defaults.length == 1 && defaults.first["model"] == "anthropic/gpt-5.6-terra"
    fail_validation("Terra must be the only default model provider")
  end

  routes = config["routing_preferences"]
  fail_validation("Plano must define exactly three routing preferences") unless routes.is_a?(Array) && routes.length == 3
  fail_validation("routing_preferences entries must be mappings") unless routes.all? { |route| route.is_a?(Hash) }

  route_names = routes.map { |route| route["name"] }
  fail_validation("routing preference names do not match the required routes") unless route_names.to_set == EXPECTED_ROUTES.keys.to_set

  routes.each do |route|
    expected = EXPECTED_ROUTES.fetch(route["name"])
    fail_validation("routing preference description does not match its mutually exclusive contract") unless route["description"] == expected["description"]
    fail_validation("routing preference must select exactly its assigned tier") unless route["models"] == [expected["model"]]
  end

  aliases = config["model_aliases"]
  fail_validation("model_aliases must be a mapping") unless aliases.is_a?(Hash)
  actual_aliases = aliases.transform_values { |value| value.is_a?(Hash) ? value["target"] : nil }
  if actual_aliases.values.any? { |target| target.to_s.include?("[1m]") }
    fail_validation("Plano alias targets must not contain [1m]")
  end
  fail_validation("Claude family aliases do not match the required tiers") unless actual_aliases == EXPECTED_ALIASES
end


def validate_cliproxy(config)
  fail_validation("CLIProxyAPI host must be 127.0.0.1") unless config["host"] == "127.0.0.1"
  fail_validation("CLIProxyAPI port must be 8317") unless config["port"] == 8317

  management = config["remote-management"]
  fail_validation("remote-management must be a mapping") unless management.is_a?(Hash)
  unless management["allow-remote"] == false && management["secret-key"].to_s.empty?
    fail_validation("management API must be disabled")
  end
  fail_validation("management control panel must be disabled") unless management["disable-control-panel"] == true

  keys = config["api-keys"]
  unless keys == ["replace-with-a-random-local-key"]
    fail_validation("api-keys must contain only the documented placeholder")
  end
end

if ARGV.length != 2
  fail_validation("usage: validate_configs.rb PLANO_CONFIG CLIPROXY_CONFIG")
end

validate_plano(load_yaml(ARGV.fetch(0), "Plano config"))
validate_cliproxy(load_yaml(ARGV.fetch(1), "CLIProxyAPI config"))
