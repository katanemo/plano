"""
Property 1: Fault Condition - Routing Header Missing Before Envoy

This test demonstrates the bug where requests to a type:model listener with failover
configuration fail with 400 error because the x-arch-llm-provider header is not set
before Envoy routing.

EXPECTED OUTCOME ON UNFIXED CODE: Test FAILS with 400 error
EXPECTED OUTCOME ON FIXED CODE: Test PASSES with successful routing
"""

import requests
import pytest
import time
import threading
from http.server import HTTPServer, BaseHTTPRequestHandler
import json


class MockProviderForExploration(BaseHTTPRequestHandler):
    """Mock provider that simulates rate limiting and successful responses"""
    
    def log_message(self, format, *args):
        """Suppress default logging"""
        pass
    
    def do_POST(self):
        port = self.server.server_port
        if port == 8082:
            # Primary provider returns 429 (rate limit)
            self.send_response(429)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(b'{"error": {"message": "Rate limit reached", "type": "requests", "code": "429"}}')
        elif port == 8083:
            # Secondary provider returns 200 (success)
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            response = {
                "id": "chatcmpl-exploration",
                "object": "chat.completion",
                "created": 1677652288,
                "model": "gpt-4o-mini",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Exploration test response",
                    },
                    "finish_reason": "stop"
                }]
            }
            self.wfile.write(json.dumps(response).encode('utf-8'))


def run_mock_server(port):
    """Run a mock server on the specified port"""
    server = HTTPServer(('0.0.0.0', port), MockProviderForExploration)
    server.serve_forever()


@pytest.fixture(scope="module", autouse=True)
def mock_servers():
    """Start mock servers for the exploration test"""
    # Start mock servers on different ports to avoid conflicts with other tests
    primary_thread = threading.Thread(target=run_mock_server, args=(8082,), daemon=True)
    secondary_thread = threading.Thread(target=run_mock_server, args=(8083,), daemon=True)
    
    primary_thread.start()
    secondary_thread.start()
    
    # Give servers time to start
    time.sleep(0.5)
    
    yield
    
    # Servers will be cleaned up automatically (daemon threads)


def test_fault_condition_routing_header_before_envoy():
    """
    Property 1: Fault Condition - Routing Header Set Before Envoy
    
    Test that requests to a type:model listener with failover configuration
    successfully route through Envoy and can execute failover logic.
    
    Bug Condition: isBugCondition(input) where:
      - input.listener_type == "model"
      - input.has_failover_config == true
      - input.routing_header_not_set_before_envoy == true
    
    Expected Behavior (after fix):
      - status_code != 400
      - request routed through Envoy successfully
      - failover executes on rate limit (primary 429 -> secondary 200)
    
    CRITICAL: This test MUST FAIL on unfixed code with 400 error
    """
    
    # NOTE: This test requires Plano to be running with tests/config_failover.yaml
    # Run: planoai up tests/config_failover.yaml --foreground
    
    try:
        response = requests.post(
            "http://localhost:12000/v1/chat/completions",
            json={
                "model": "openai/gpt-4",
                "messages": [{"role": "user", "content": "Test routing header"}]
            },
            timeout=10
        )
        
        # Document the counterexample
        print(f"\n=== Exploration Test Results ===")
        print(f"Status Code: {response.status_code}")
        print(f"Response Headers: {dict(response.headers)}")
        print(f"Response Body: {response.text[:200]}")
        
        # Expected behavior after fix:
        # 1. Request should NOT return 400 (header should be set before Envoy)
        assert response.status_code != 400, (
            f"BUG CONFIRMED: Got 400 error, likely 'x-arch-llm-provider header not set'. "
            f"This confirms the header is not set before Envoy routing. "
            f"Response: {response.text}"
        )
        
        # 2. Request should succeed (either 200 from primary or 200 from secondary after failover)
        assert response.status_code == 200, (
            f"Expected 200 after successful routing and potential failover, got {response.status_code}. "
            f"Response: {response.text}"
        )
        
        # 3. Response should contain valid completion
        response_json = response.json()
        assert "choices" in response_json, "Response should contain choices"
        assert len(response_json["choices"]) > 0, "Response should have at least one choice"
        
        print(f"✅ TEST PASSED: Routing header set correctly, failover executed successfully")
        
    except requests.exceptions.ConnectionError:
        pytest.skip("Plano is not running. Start with: planoai up tests/config_failover.yaml --foreground")
    except AssertionError as e:
        # This is expected on unfixed code
        print(f"\n❌ COUNTEREXAMPLE FOUND: {str(e)}")
        print(f"This confirms the bug exists - the x-arch-llm-provider header is not set before Envoy routing")
        raise


if __name__ == "__main__":
    # Allow running directly for manual testing
    print("Starting exploration test...")
    print("Make sure Plano is running: planoai up tests/config_failover.yaml --foreground")
    print()
    
    # Documented counterexample from bugfix.md:
    # Request to http://localhost:12000/v1/chat/completions with model openai/gpt-4
    # Returns: 400 "x-arch-llm-provider header not set, llm gateway cannot perform routing"
    # This confirms the bug exists - header is not set before Envoy routing
    
    # Run the test
    test_fault_condition_routing_header_before_envoy()
