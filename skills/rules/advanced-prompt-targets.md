---
title: Design Prompt Targets with Precise Parameter Schemas
impact: HIGH
impactDescription: Imprecise parameter definitions cause the LLM to hallucinate values, skip required fields, or produce malformed API calls — the schema is the contract between the LLM and your API
tags: advanced, prompt-targets, functions, llm, api-integration
---

## Design Prompt Targets with Precise Parameter Schemas

`prompt_targets` define functions that Plano's LLM can call autonomously when it determines a user request matches the function's description. The parameter schema tells the LLM exactly what values to extract from user input — vague schemas lead to hallucinated parameters and failed API calls.

**Incorrect (too few constraints — LLM must guess):**

```yaml
prompt_targets:
  - name: get_flight_info
    description: Get flight information
    parameters:
      - name: flight         # What format? "AA123"? "AA 123"? "American 123"?
        type: str
        required: true
    endpoint:
      name: flights_api
      path: /flight?id={flight}
```

**Correct (fully specified schema with descriptions, formats, and enums):**

```yaml
version: v0.3.0

endpoints:
  flights_api:
    endpoint: api.flightaware.com
    protocol: https
    connect_timeout: "5s"

prompt_targets:
  - name: get_flight_status
    description: >
      Get real-time status, gate information, and delays for a specific flight number.
      Use when the user asks about a flight's current status, arrival time, or gate.
    parameters:
      - name: flight_number
        description: >
          IATA airline code followed by flight number, e.g., "AA123", "UA456", "DL789".
          Extract from user message — do not include spaces.
        type: str
        required: true
        format: "^[A-Z]{2}[0-9]{1,4}$"    # Regex hint for validation

      - name: date
        description: >
          Flight date in YYYY-MM-DD format. Use today's date if not specified.
        type: str
        required: false
        format: date

    endpoint:
      name: flights_api
      path: /flights/{flight_number}?date={date}
      http_method: GET
      http_headers:
        Authorization: "Bearer $FLIGHTAWARE_API_KEY"

  - name: search_flights
    description: >
      Search for available flights between two cities or airports.
      Use when the user wants to find flights, compare options, or book travel.
    parameters:
      - name: origin
        description: Departure airport IATA code (e.g., "JFK", "LAX", "ORD")
        type: str
        required: true
      - name: destination
        description: Arrival airport IATA code (e.g., "LHR", "CDG", "NRT")
        type: str
        required: true
      - name: departure_date
        description: Departure date in YYYY-MM-DD format
        type: str
        required: true
        format: date
      - name: cabin_class
        description: Preferred cabin class
        type: str
        required: false
        default: economy
        enum: [economy, premium_economy, business, first]
      - name: passengers
        description: Number of adult passengers (1-9)
        type: int
        required: false
        default: 1

    endpoint:
      name: flights_api
      path: /search?from={origin}&to={destination}&date={departure_date}&class={cabin_class}&pax={passengers}
      http_method: GET
      http_headers:
        Authorization: "Bearer $FLIGHTAWARE_API_KEY"

    system_prompt: |
      You are a travel assistant. Present flight search results clearly,
      highlighting the best value options. Include price, duration, and
      number of stops for each option.

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

listeners:
  - type: prompt
    name: travel_functions
    port: 10000
    timeout: "30s"
```

**Key principles:**
- `description` on the target tells the LLM when to call it — be specific about trigger conditions
- `description` on each parameter tells the LLM what value to extract — include format examples
- Use `enum` to constrain categorical values — prevents the LLM from inventing categories
- Use `format: date` or regex patterns to hint at expected format
- Use `default` for optional parameters so the API never receives null values
- `system_prompt` on the target customizes how the LLM formats the API response to the user

Reference: https://github.com/katanemo/archgw
