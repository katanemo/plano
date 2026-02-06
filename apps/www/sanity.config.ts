import { defineConfig } from "sanity";
import { codeInput } from "@sanity/code-input";
import { table } from "@sanity/table";
import { markdownSchema } from "sanity-plugin-markdown";
import { structureTool } from "sanity/structure";
import { schemaTypes } from "./schemaTypes";

export default defineConfig({
  name: "default",
  title: "Plano",

  projectId: "71ny25bn",
  dataset: "production",

  basePath: "/studio",

  plugins: [structureTool(), codeInput(), table(), markdownSchema()],

  schema: {
    types: schemaTypes,
  },
});
