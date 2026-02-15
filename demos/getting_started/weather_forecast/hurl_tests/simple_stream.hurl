POST http://localhost:10000/v1/chat/completions
Content-Type: application/json

{
  "messages": [
    {
      "role": "user",
      "content": "how is the weather in seattle for next 5 days"
    }
  ],
  "stream": true
}
HTTP 200
[Asserts]
header "content-type" matches /text\/event-stream/
body matches "(?s).*\"name\":\"get_current_weather\".*"
body matches "(?s).*\"model\":\"gpt-4o-mini.*"
