import { createOpenAI } from "@ai-sdk/openai";
import {
  extractReasoningMiddleware,
  type LanguageModel,
  wrapLanguageModel,
} from "ai";

const plano = createOpenAI({
  baseURL: process.env.PLANO_BASE_URL || "http://localhost:12000/v1",
  apiKey: "plano",
});

const THINKING_SUFFIX_REGEX = /-thinking$/;

type WrapLanguageModelInput = Parameters<typeof wrapLanguageModel>[0]["model"];

function asLanguageModel(model: unknown): LanguageModel {
  // We intentionally cast here to avoid TS conflicts when multiple copies of
  // `@ai-sdk/provider` exist in node_modules (e.g. nested under @ai-sdk/openai).
  return model as unknown as LanguageModel;
}

function asWrapLanguageModelInput(model: unknown): WrapLanguageModelInput {
  return model as unknown as WrapLanguageModelInput;
}

export function getLanguageModel(modelId: string): LanguageModel {
  const isReasoningModel =
    modelId.includes("reasoning") || modelId.endsWith("-thinking");

  if (isReasoningModel) {
    const gatewayModelId = modelId.replace(THINKING_SUFFIX_REGEX, "");

    return asLanguageModel(
      wrapLanguageModel({
        model: asWrapLanguageModelInput(plano(gatewayModelId)),
        middleware: extractReasoningMiddleware({ tagName: "thinking" }),
      })
    );
  }

  return asLanguageModel(plano(modelId));
}

export function getTitleModel(): LanguageModel {
  // Keep demo dependency-light: default to an OpenAI model so only OPENAI_API_KEY is required.
  return asLanguageModel(plano("openai/gpt-4.1-mini"));
}
