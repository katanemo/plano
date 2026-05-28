import json
import os

import httpx
import streamlit as st

# Configuration
PLANO_ENDPOINT = os.getenv("PLANO_ENDPOINT", "http://localhost:8001/v1")

st.set_page_config(
    page_title="Credit Risk Case Copilot",
    page_icon="🏦",
    layout="wide",
    initial_sidebar_state="expanded",
)


# Load scenarios
def load_scenario(scenario_file: str):
    """Load scenario JSON from file."""
    try:
        with open(scenario_file, "r") as f:
            return json.load(f)
    except FileNotFoundError:
        return None


def extract_json_block(content: str):
    """Extract the first JSON block from an agent response."""
    try:
        if "```json" in content:
            json_start = content.index("```json") + 7
            json_end = content.index("```", json_start)
            return json.loads(content[json_start:json_end].strip())
        if "{" in content and "}" in content:
            json_start = content.index("{")
            json_end = content.rindex("}") + 1
            return json.loads(content[json_start:json_end].strip())
    except Exception:
        return None
    return None


def call_plano(application_data: dict):
    """Call Plano once and return the assistant content and parsed JSON block."""
    response = httpx.post(
        f"{PLANO_ENDPOINT}/chat/completions",
        json={
            "model": "risk_reasoning",
            "messages": [
                {
                    "role": "user",
                    "content": (
                        "Run the full credit risk pipeline: intake -> risk -> policy -> memo. "
                        "Return the final decision memo for the applicant and include JSON.\n\n"
                        f"{json.dumps(application_data, indent=2)}"
                    ),
                }
            ],
        },
        timeout=90.0,
    )
    if response.status_code != 200:
        return None, None, {
            "status_code": response.status_code,
            "text": response.text,
        }

    raw = response.json()
    content = raw["choices"][0]["message"]["content"]
    parsed = extract_json_block(content)
    return content, parsed, raw


# Initialize session state
if "assistant_content" not in st.session_state:
    st.session_state.assistant_content = None
if "parsed_result" not in st.session_state:
    st.session_state.parsed_result = None
if "raw_response" not in st.session_state:
    st.session_state.raw_response = None
if "application_json" not in st.session_state:
    st.session_state.application_json = "{}"


# Header
st.title("🏦 Credit Risk Case Copilot")
st.markdown("A minimal UI for the Plano + CrewAI credit risk demo.")
st.divider()

# Sidebar
with st.sidebar:
    st.header("📋 Loan Application Input")

    # Scenario selection
    st.subheader("Quick Scenarios")
    col1, col2, col3 = st.columns(3)

    if col1.button("🟢 A\nLow", use_container_width=True):
        scenario = load_scenario("scenarios/scenario_a_low_risk.json")
        if scenario:
            st.session_state.application_json = json.dumps(scenario, indent=2)

    if col2.button("🟡 B\nMedium", use_container_width=True):
        scenario = load_scenario("scenarios/scenario_b_medium_risk.json")
        if scenario:
            st.session_state.application_json = json.dumps(scenario, indent=2)

    if col3.button("🔴 C\nHigh", use_container_width=True):
        scenario = load_scenario("scenarios/scenario_c_high_risk_injection.json")
        if scenario:
            st.session_state.application_json = json.dumps(scenario, indent=2)

    st.divider()

    # JSON input area
    application_json = st.text_area(
        "Loan Application JSON",
        value=st.session_state.application_json,
        height=380,
        help="Paste or edit loan application JSON",
    )

    col_a, col_b = st.columns(2)

    with col_a:
        if st.button("🔍 Assess Risk", type="primary", use_container_width=True):
            try:
                application_data = json.loads(application_json)
                with st.spinner("Running credit risk assessment..."):
                    content, parsed, raw = call_plano(application_data)

                if content is None:
                    st.session_state.assistant_content = None
                    st.session_state.parsed_result = None
                    st.session_state.raw_response = raw
                    st.error("Request failed. See raw response for details.")
                else:
                    st.session_state.assistant_content = content
                    st.session_state.parsed_result = parsed
                    st.session_state.raw_response = raw
                    st.success("✅ Risk assessment complete!")

            except json.JSONDecodeError:
                st.error("Invalid JSON format")
            except Exception as e:
                st.error(f"Error: {str(e)}")

    with col_b:
        if st.button("🧹 Clear", use_container_width=True):
            st.session_state.assistant_content = None
            st.session_state.parsed_result = None
            st.session_state.raw_response = None
            st.session_state.application_json = "{}"
            st.rerun()


# Main content area
if st.session_state.assistant_content or st.session_state.parsed_result:
    parsed = st.session_state.parsed_result or {}

    st.header("Decision")

    col1, col2 = st.columns(2)

    with col1:
        st.metric(
            "Recommended Action",
            parsed.get("recommended_action", "REVIEW"),
        )

    with col2:
        st.metric("Step", parsed.get("step", "memo"))

    st.divider()

    st.subheader("Decision Memo")
    memo = parsed.get("decision_memo") or st.session_state.assistant_content
    if memo:
        st.markdown(memo)
    else:
        st.info("No decision memo available.")

    with st.expander("Raw Response"):
        st.json(st.session_state.raw_response or {})

else:
    st.info(
        "👈 Select a scenario or paste a loan application JSON in the sidebar, then click **Assess Risk**."
    )

    st.subheader("Sample Application Format")
    st.code(
        """{
  "applicant_name": "John Doe",
  "loan_amount": 500000,
  "monthly_income": 150000,
  "employment_status": "FULL_TIME",
  "employment_duration_months": 36,
  "credit_score": 720,
  "existing_loans": 1,
  "total_debt": 45000,
  "delinquencies": 0,
  "utilization_rate": 35.5,
  "kyc_complete": true,
  "income_verified": true,
  "address_verified": true
}""",
        language="json",
    )
