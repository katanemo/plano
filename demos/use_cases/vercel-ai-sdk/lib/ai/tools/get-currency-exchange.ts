import { tool } from "ai";
import { z } from "zod";

export const getCurrencyExchange = tool({
  description:
    "Get current exchange rates between currencies using the Frankfurter API. You can convert between different currencies and get the latest exchange rates.",
  inputSchema: z.object({
    from: z
      .string()
      .describe(
        "The base currency code (e.g., 'USD', 'EUR', 'GBP'). Defaults to USD if not provided."
      )
      .optional(),
    to: z
      .string()
      .describe(
        "The target currency code to convert to (e.g., 'USD', 'EUR', 'GBP'). If not provided, returns rates for all available currencies."
      )
      .optional(),
    amount: z
      .number()
      .describe("The amount to convert. Defaults to 1 if not provided.")
      .optional(),
  }),
  needsApproval: true,
  execute: async (input) => {
    const from = input.from?.toUpperCase() || "USD";
    const amount = input.amount || 1;
    const to = input.to?.toUpperCase();

    try {
      // Build the API URL
      let url = `https://api.frankfurter.dev/v1/latest?base=${from}`;
      if (to) {
        url += `&symbols=${to}`;
      }

      const response = await fetch(url);

      if (!response.ok) {
        return {
          error: `Failed to fetch exchange rates. Please check the currency codes and try again.`,
        };
      }

      const data = await response.json();

      // Calculate converted amounts if amount is provided
      if (data.rates) {
        const convertedRates: Record<string, number> = {};
        for (const [currency, rate] of Object.entries(data.rates)) {
          convertedRates[currency] = Number(
            (amount * (rate as number)).toFixed(2)
          );
        }

        return {
          base: data.base,
          date: data.date,
          amount,
          rates: data.rates,
          convertedRates,
        };
      }

      return data;
    } catch {
      return {
        error:
          "An error occurred while fetching exchange rates. Please try again.",
      };
    }
  },
});
