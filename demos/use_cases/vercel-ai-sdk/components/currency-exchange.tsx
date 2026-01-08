"use client";

import { format } from "date-fns";
import { ArrowRight, TrendingUp } from "lucide-react";

const CurrencyIcon = ({ currency }: { currency: string }) => {
  const symbols: Record<string, string> = {
    USD: "$",
    EUR: "€",
    GBP: "£",
    JPY: "¥",
    CHF: "₣",
    CAD: "$",
    AUD: "$",
    CNY: "¥",
    INR: "₹",
    BRL: "R$",
    KRW: "₩",
    MXN: "$",
    RUB: "₽",
    ZAR: "R",
    TRY: "₺",
    SEK: "kr",
    NOK: "kr",
    DKK: "kr",
    PLN: "zł",
  };

  return (
    <div className="flex size-10 items-center justify-center rounded-full bg-gradient-to-br from-emerald-500 to-teal-600 font-bold text-lg text-white shadow-md">
      {symbols[currency] || currency.substring(0, 1)}
    </div>
  );
};

type CurrencyExchangeData = {
  base: string;
  date: string;
  amount: number;
  rates: Record<string, number>;
  convertedRates: Record<string, number>;
};

const SAMPLE: CurrencyExchangeData = {
  base: "USD",
  date: "2026-01-08",
  amount: 100,
  rates: {
    EUR: 0.8565,
    GBP: 0.7874,
    JPY: 111.5,
    CHF: 0.8542,
    CAD: 1.3456,
  },
  convertedRates: {
    EUR: 85.65,
    GBP: 78.74,
    JPY: 11150,
    CHF: 85.42,
    CAD: 134.56,
  },
};

function formatCurrencyValue(value: number): string {
  if (value >= 1000) {
    return value.toLocaleString("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });
  }
  return value.toFixed(2);
}

export function CurrencyExchange({
  exchangeData = SAMPLE,
}: {
  exchangeData?: CurrencyExchangeData;
}) {
  const currencies = Object.keys(exchangeData.rates).slice(0, 6);
  const isSingleConversion = currencies.length === 1;

  return (
    <div className="relative flex w-full flex-col gap-3 overflow-hidden rounded-2xl bg-gradient-to-br from-emerald-50 via-teal-50 to-cyan-50 p-4 shadow-lg dark:from-emerald-950/40 dark:via-teal-950/40 dark:to-cyan-950/40">
      <div className="absolute inset-0 bg-gradient-to-br from-emerald-500/5 to-teal-500/5" />

      <div className="relative z-10">
        <div className="mb-2 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <TrendingUp className="size-4 text-emerald-600 dark:text-emerald-400" />
            <div className="font-semibold text-emerald-900 text-sm dark:text-emerald-100">
              Currency Exchange
            </div>
          </div>
          <div className="text-emerald-700 text-xs dark:text-emerald-300">
            {format(new Date(exchangeData.date), "MMM d, yyyy")}
          </div>
        </div>

        {isSingleConversion ? (
          <div className="rounded-xl bg-white/80 p-4 shadow-sm backdrop-blur-sm dark:bg-gray-900/80">
            <div className="mb-3 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <CurrencyIcon currency={exchangeData.base} />
                <div>
                  <div className="font-semibold text-gray-900 text-sm dark:text-gray-100">
                    {exchangeData.base}
                  </div>
                  <div className="font-light text-2xl text-gray-900 dark:text-gray-100">
                    {formatCurrencyValue(exchangeData.amount)}
                  </div>
                </div>
              </div>
              <ArrowRight className="size-5 text-emerald-600 dark:text-emerald-400" />
              <div className="flex items-center gap-3">
                <CurrencyIcon currency={currencies[0]} />
                <div className="text-right">
                  <div className="font-semibold text-gray-900 text-sm dark:text-gray-100">
                    {currencies[0]}
                  </div>
                  <div className="font-light text-2xl text-emerald-600 dark:text-emerald-400">
                    {formatCurrencyValue(
                      exchangeData.convertedRates[currencies[0]]
                    )}
                  </div>
                </div>
              </div>
            </div>
            <div className="border-t pt-3 text-center text-gray-600 text-xs dark:text-gray-400">
              Exchange Rate: 1 {exchangeData.base} ={" "}
              {exchangeData.rates[currencies[0]].toFixed(4)} {currencies[0]}
            </div>
          </div>
        ) : (
          <>
            <div className="mb-3 flex items-center gap-3">
              <CurrencyIcon currency={exchangeData.base} />
              <div>
                <div className="font-medium text-gray-700 text-xs dark:text-gray-300">
                  Base Currency
                </div>
                <div className="font-light text-2xl text-gray-900 dark:text-gray-100">
                  {formatCurrencyValue(exchangeData.amount)}{" "}
                  <span className="font-semibold text-lg">
                    {exchangeData.base}
                  </span>
                </div>
              </div>
            </div>

            <div className="rounded-xl bg-white/80 p-3 backdrop-blur-sm dark:bg-gray-900/80">
              <div className="mb-2 font-medium text-gray-700 text-xs dark:text-gray-300">
                Exchange Rates
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                {currencies.map((currency) => (
                  <div
                    className="flex items-center justify-between rounded-lg bg-gradient-to-r from-emerald-50 to-teal-50 p-2.5 dark:from-emerald-950/50 dark:to-teal-950/50"
                    key={currency}
                  >
                    <div className="flex items-center gap-2">
                      <div className="flex size-7 items-center justify-center rounded-full bg-gradient-to-br from-emerald-500 to-teal-600 font-semibold text-white text-xs">
                        {currency.substring(0, 1)}
                      </div>
                      <span className="font-medium text-gray-900 text-sm dark:text-gray-100">
                        {currency}
                      </span>
                    </div>
                    <div className="text-right">
                      <div className="font-semibold text-emerald-600 text-sm dark:text-emerald-400">
                        {formatCurrencyValue(
                          exchangeData.convertedRates[currency]
                        )}
                      </div>
                      <div className="text-gray-500 text-xs dark:text-gray-400">
                        {exchangeData.rates[currency].toFixed(4)}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </>
        )}

        <div className="mt-2 text-center text-emerald-700 text-xs dark:text-emerald-300">
          Powered by Frankfurter API
        </div>
      </div>
    </div>
  );
}
