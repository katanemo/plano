POST http://localhost:12000/v1/chat/completions
Content-Type: application/json

{
  "model": "openai/gpt-4o-mini",
  "messages": [
    {
      "role": "user",
      "content": "hi"
    }
  ]
}
HTTP 200
[Asserts]
header "content-type" == "application/json"
jsonpath "$.model" matches /^gpt-4o-mini/
jsonpath "$.usage" != null
jsonpath "$.choices[0].message.content" != null
jsonpath "$.choices[0].message.role" == "assistant"
