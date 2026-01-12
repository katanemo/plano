import type { InferUITool, UIMessage } from "ai";
import { z } from "zod";
import type { getWeather } from "./ai/tools/get-weather";
import type { getCurrencyExchange } from "./ai/tools/get-currency-exchange";

export type DataPart = { type: "append-message"; message: string };

export const messageMetadataSchema = z.object({
  createdAt: z.string(),
});

export type MessageMetadata = z.infer<typeof messageMetadataSchema>;

type weatherTool = InferUITool<typeof getWeather>;
type currencyExchangeTool = InferUITool<typeof getCurrencyExchange>;

export type ChatTools = {
  getWeather: weatherTool;
  getCurrencyExchange: currencyExchangeTool;
};

export type CustomUIDataTypes = {
  appendMessage: string;
  "chat-title": string;
};

export type ChatMessage = UIMessage<
  MessageMetadata,
  CustomUIDataTypes,
  ChatTools
>;

export type Attachment = {
  name: string;
  url: string;
  contentType: string;
};
