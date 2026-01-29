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


def call_plano(step_label: str, payload: dict):
    """Call Plano and return the parsed JSON response."""
    response = httpx.post(
        f"{PLANO_ENDPOINT}/chat/completions",
        json={
            "model": "risk_reasoning",
            "messages": [
                {
                    "role": "user",
                    "content": (
                        f"Run the {step_label} step only. Return JSON.\n\n"
                        f"{json.dumps(payload, indent=2)}"
                    ),
                }
            ],
        },
        timeout=60.0,
    )
    if response.status_code != 200:
        return None, {
            "status_code": response.status_code,
            "text": response.text,
        }

    raw = response.json()
    content = raw["choices"][0]["message"]["content"]
    parsed = extract_json_block(content)
    return parsed, raw


# Initialize session state
if "workflow_result" not in st.session_state:
    st.session_state.workflow_result = None
if "raw_results" not in st.session_state:
    st.session_state.raw_results = {}
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

                with st.spinner("Running intake..."):
                    intake, intake_raw = call_plano("loan intake normalization", application_data)
                if not intake:
                    st.session_state.workflow_result = None
                    st.session_state.raw_results = {"intake": intake_raw}
                    st.error("Intake step failed.")
                    st.stop()

                with st.spinner("Running risk scoring..."):
                    risk_payload = {"application": application_data, "intake": intake}
                    risk, risk_raw = call_plano("risk scoring", risk_payload)
                if not risk:
                    st.session_state.workflow_result = None
                    st.session_state.raw_results = {
                        "intake": intake_raw,
                        "risk": risk_raw,
                    }
                    st.error("Risk scoring step failed.")
                    st.stop()

                with st.spinner("Running policy compliance..."):
                    policy_payload = {
                        "application": application_data,
                        "intake": intake,
                        "risk": risk,
                    }
                    policy, policy_raw = call_plano("policy compliance", policy_payload)
                if not policy:
                    st.session_state.workflow_result = None
                    st.session_state.raw_results = {
                        "intake": intake_raw,
                        "risk": risk_raw,
                        "policy": policy_raw,
                    }
                    st.error("Policy compliance step failed.")
                    st.stop()

                with st.spinner("Running decision memo..."):
                    memo_payload = {
                        "application": application_data,
                        "intake": intake,
                        "risk": risk,
                        "policy": policy,
                    }
                    memo, memo_raw = call_plano("decision memo", memo_payload)
                if not memo:
                    st.session_state.workflow_result = None
                    st.session_state.raw_results = {
                        "intake": intake_raw,
                        "risk": risk_raw,
                        "policy": policy_raw,
                        "memo": memo_raw,
                    }
                    st.error("Decision memo step failed.")
                    st.stop()

                st.session_state.workflow_result = {
                    "application": application_data,
                    "intake": intake,
                    "risk": risk,
                    "policy": policy,
                    "memo": memo,
                }
                st.session_state.raw_results = {
                    "intake": intake_raw,
                    "risk": risk_raw,
                    "policy": policy_raw,
                    "memo": memo_raw,
                }
                st.success("✅ Risk assessment complete!")

            except json.JSONDecodeError:
                st.error("Invalid JSON format")
            except Exception as e:
                st.error(f"Error: {str(e)}")

    with col_b:
        if st.button("🧹 Clear", use_container_width=True):
            st.session_state.workflow_result = None
            st.session_state.raw_results = {}
            st.session_state.application_json = "{}"
            st.rerun()


# Main content area
if st.session_state.workflow_result:
    result = st.session_state.workflow_result

    st.header("Decision")

    col1, col2, col3 = st.columns(3)

    with col1:
        risk_color = {"LOW": "🟢", "MEDIUM": "🟡", "HIGH": "🔴"}
        risk_band = result.get("risk", {}).get("risk_band", "UNKNOWN")
        st.metric("Risk Band", f"{risk_color.get(risk_band, '⚪')} {risk_band}")

    with col2:
        confidence = result.get("risk", {}).get("confidence_score", 0.0)
        try:
            st.metric("Confidence", f"{float(confidence):.0%}")
        except Exception:
            st.metric("Confidence", str(confidence))

    with col3:
        st.metric(
            "Recommended Action",
            result.get("memo", {}).get("recommended_action", "REVIEW"),
        )

    st.divider()

    st.subheader("Decision Memo")
    memo = result.get("memo", {}).get("decision_memo", "")
    if memo:
        st.markdown(memo)
    else:
        st.info("No decision memo available.")

    st.divider()
    with st.expander("Normalized Application"):
        st.json(result.get("intake", {}).get("normalized_data", {}))

    with st.expander("Step Outputs (debug)"):
        st.json(st.session_state.raw_results or {})

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
