---
title: Generate Prompt Targets from Python Functions with `planoai generate_prompt_targets`
impact: MEDIUM
impactDescription: Manually writing prompt_targets YAML for existing Python APIs is error-prone — the generator introspects function signatures and produces correct YAML automatically
tags: cli, generate, prompt-targets, python, code-generation
---

## Generate Prompt Targets from Python Functions with `planoai generate_prompt_targets`

`planoai generate_prompt_targets` introspects Python function signatures and docstrings to generate `prompt_targets` YAML for your Plano config. This is the fastest way to expose existing Python APIs as LLM-callable functions without manually writing the YAML schema.

**Python function requirements for generation:**
- Use simple type annotations: `int`, `float`, `bool`, `str`, `list`, `tuple`, `set`, `dict`
- Include a docstring describing what the function does (becomes the `description`)
- Complex Pydantic models must be flattened into primitive typed parameters first

**Example Python file:**

```python
# api.py

def get_stock_quote(symbol: str, exchange: str = "NYSE") -> dict:
    """Get the current stock price and trading data for a given stock symbol.

    Returns price, volume, market cap, and 24h change percentage.
    """
    # Implementation calls stock API
    pass

def get_weather_forecast(city: str, days: int = 3, units: str = "celsius") -> dict:
    """Get the weather forecast for a city.

    Returns temperature, precipitation, and conditions for the specified number of days.
    """
    pass

def search_flights(origin: str, destination: str, date: str, passengers: int = 1) -> list:
    """Search for available flights between two airports on a given date.

    Date format: YYYY-MM-DD. Returns list of flight options with prices.
    """
    pass
```

**Running the generator:**

```bash
planoai generate_prompt_targets --file api.py
```

**Generated output (add to your config.yaml):**

```yaml
prompt_targets:
  - name: get_stock_quote
    description: Get the current stock price and trading data for a given stock symbol.
    parameters:
      - name: symbol
        type: str
        required: true
      - name: exchange
        type: str
        required: false
        default: NYSE
    # Add endpoint manually:
    endpoint:
      name: stock_api
      path: /quote?symbol={symbol}&exchange={exchange}

  - name: get_weather_forecast
    description: Get the weather forecast for a city.
    parameters:
      - name: city
        type: str
        required: true
      - name: days
        type: int
        required: false
        default: 3
      - name: units
        type: str
        required: false
        default: celsius
    endpoint:
      name: weather_api
      path: /forecast?city={city}&days={days}&units={units}
```

After generation, manually add the `endpoint` blocks pointing to your actual API. The generator produces the schema; you wire in the connectivity.

Reference: https://github.com/katanemo/archgw
