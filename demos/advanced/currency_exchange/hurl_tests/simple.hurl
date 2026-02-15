POST http://localhost:10000/v1/chat/completions
Content-Type: application/json

{
  "messages": [
    {
      "role": "user",
      "content": "convert 100 eur"
    }
  ],
  "model": "gpt-4o"
}
HTTP 200
[Asserts]
header "content-type" == "application/json"
jsonpath "$.model" matches /^gpt-4o/
jsonpath "$.metadata.x-arch-state" != null
jsonpath "$.usage" != null
jsonpath "$.choices[0].message.content" != null
jsonpath "$.choices[0].message.role" == "assistant"
