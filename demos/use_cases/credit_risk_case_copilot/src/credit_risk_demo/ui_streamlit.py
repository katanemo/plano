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


# Initialize session state
if "assessment_result" not in st.session_state:
    st.session_state.assessment_result = None
if "raw_result" not in st.session_state:
    st.session_state.raw_result = None
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

                with st.spinner("Running risk assessment..."):
                    response = httpx.post(
                        f"{PLANO_ENDPOINT}/chat/completions",
                        json={
                            # Use risk_reasoning if you’re standardizing on aliases.
                            # If you want plain OpenAI model routing, set "gpt-4o".
                            "model": "risk_reasoning",
                            "messages": [
                                {
                                    "role": "user",
                                    "content": (
                                        "Assess credit risk for this loan application:\n\n"
                                        f"{json.dumps(application_data, indent=2)}"
                                    ),
                                }
                            ],
                        },
                        timeout=60.0,
                    )

                if response.status_code == 200:
                    raw = response.json()
                    st.session_state.raw_result = raw

                    content = raw["choices"][0]["message"]["content"]

                    # Extract JSON block from response
                    if "```json" in content:
                        json_start = content.index("```json") + 7
                        json_end = content.index("```", json_start)
                        json_str = content[json_start:json_end].strip()
                        assessment = json.loads(json_str)
                        st.session_state.assessment_result = assessment
                        st.success("✅ Risk assessment complete!")
                    else:
                        st.session_state.assessment_result = None
                        st.error(
                            "Could not parse JSON assessment from the agent response."
                        )
                else:
                    st.session_state.assessment_result = None
                    st.session_state.raw_result = {
                        "status_code": response.status_code,
                        "text": response.text,
                    }
                    st.error(f"Error: {response.status_code} - {response.text}")

            except json.JSONDecodeError:
                st.error("Invalid JSON format")
            except Exception as e:
                st.error(f"Error: {str(e)}")

    with col_b:
        if st.button("🧹 Clear", use_container_width=True):
            st.session_state.assessment_result = None
            st.session_state.raw_result = None
            st.session_state.application_json = "{}"
            st.rerun()


# Main content area
if st.session_state.assessment_result:
    result = st.session_state.assessment_result

    st.header("Decision")

    col1, col2, col3 = st.columns(3)

    with col1:
        risk_color = {"LOW": "🟢", "MEDIUM": "🟡", "HIGH": "🔴"}
        risk_band = result.get("risk_band", "UNKNOWN")
        st.metric("Risk Band", f"{risk_color.get(risk_band, '⚪')} {risk_band}")

    with col2:
        confidence = result.get("confidence", 0.0)
        try:
            st.metric("Confidence", f"{float(confidence):.0%}")
        except Exception:
            st.metric("Confidence", str(confidence))

    with col3:
        st.metric("Recommended Action", result.get("recommended_action", "REVIEW"))

    st.divider()

    tab1, tab2, tab3 = st.tabs(["🧾 Summary", "📝 Decision Memo", "🧪 Raw Output"])

    with tab1:
        st.subheader("Summary")

        human = result.get("human_response", "")
        if human:
            st.write(human.split("```")[0].strip())
        else:
            st.info("No human-readable summary available.")

        st.divider()
        st.subheader("Normalized Application")
        st.json(result.get("normalized_application", {}))

    with tab2:
        st.subheader("Decision Memo")
        memo = result.get("decision_memo", "")
        if memo:
            st.markdown(memo)
        else:
            st.info("No decision memo available.")

    with tab3:
        st.subheader("Raw Output")
        with st.expander("Show raw agent response JSON"):
            st.json(st.session_state.raw_result or {})

        with st.expander("Show parsed assessment JSON"):
            st.json(result)

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
