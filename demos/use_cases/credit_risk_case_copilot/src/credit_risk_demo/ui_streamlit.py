import json
import os
from datetime import datetime

import httpx
import streamlit as st

# Configuration
PLANO_ENDPOINT = os.getenv("PLANO_ENDPOINT", "http://localhost:8001/v1")
CASE_SERVICE_URL = "http://localhost:10540"

st.set_page_config(
    page_title="Credit Risk Case Copilot",
    page_icon="🏦",
    layout="wide",
    initial_sidebar_state="expanded",
)


# Load scenarios
def load_scenario(scenario_file):
    """Load scenario JSON from file."""
    try:
        with open(scenario_file, "r") as f:
            return json.load(f)
    except FileNotFoundError:
        return None


# Initialize session state
if "assessment_result" not in st.session_state:
    st.session_state.assessment_result = None
if "case_id" not in st.session_state:
    st.session_state.case_id = None


# Header
st.title("🏦 Credit Risk Case Copilot")
st.markdown("**AI-Powered Credit Risk Assessment & Case Management**")
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
        value=st.session_state.get("application_json", "{}"),
        height=400,
        help="Paste or edit loan application JSON",
    )

    # Assess button
    if st.button("🔍 Assess Risk", type="primary", use_container_width=True):
        try:
            # Parse JSON
            application_data = json.loads(application_json)

            # Call Plano orchestrator
            with st.spinner("Running risk assessment..."):
                response = httpx.post(
                    f"{PLANO_ENDPOINT}/chat/completions",
                    json={
                        "model": "gpt-4o",
                        "messages": [
                            {
                                "role": "user",
                                "content": f"Assess credit risk for this loan application:\n\n{json.dumps(application_data, indent=2)}",
                            }
                        ],
                    },
                    timeout=60.0,
                )

                if response.status_code == 200:
                    result = response.json()
                    content = result["choices"][0]["message"]["content"]

                    # Extract JSON from response
                    if "```json" in content:
                        json_start = content.index("```json") + 7
                        json_end = content.index("```", json_start)
                        json_str = content[json_start:json_end].strip()
                        assessment = json.loads(json_str)
                        st.session_state.assessment_result = assessment
                        st.success("✅ Risk assessment complete!")
                    else:
                        st.error("Could not parse assessment result")
                else:
                    st.error(f"Error: {response.status_code} - {response.text}")

        except json.JSONDecodeError:
            st.error("Invalid JSON format")
        except Exception as e:
            st.error(f"Error: {str(e)}")

# Main content area
if st.session_state.assessment_result:
    result = st.session_state.assessment_result

    # Risk summary
    st.header("Risk Assessment Summary")

    col1, col2, col3, col4 = st.columns(4)

    with col1:
        risk_color = {"LOW": "🟢", "MEDIUM": "🟡", "HIGH": "🔴"}
        st.metric(
            "Risk Band",
            f"{risk_color.get(result['risk_band'], '⚪')} {result['risk_band']}",
        )

    with col2:
        st.metric("Confidence", f"{result['confidence']:.1%}")

    with col3:
        st.metric("Recommended Action", result["recommended_action"])

    with col4:
        st.metric("Documents Required", len(result.get("required_documents", [])))

    st.divider()

    # Tabbed interface
    tab1, tab2, tab3, tab4, tab5 = st.tabs(
        [
            "📊 Risk Summary",
            "🎯 Risk Drivers",
            "📋 Policy & Compliance",
            "📝 Decision Memo",
            "🔍 Audit Trail",
        ]
    )

    with tab1:
        st.subheader("Normalized Application")
        st.json(result.get("normalized_application", {}))

        st.subheader("Assessment Overview")
        st.write(result.get("human_response", "").split("```")[0])

    with tab2:
        st.subheader("Risk Drivers")
        drivers = result.get("drivers", [])

        for driver in drivers:
            impact_color = {"CRITICAL": "🔴", "HIGH": "🟠", "MEDIUM": "🟡", "LOW": "🟢"}
            st.markdown(
                f"**{impact_color.get(driver['impact'], '⚪')} {driver['factor']}** ({driver['impact']})"
            )
            st.write(driver["evidence"])
            st.divider()

    with tab3:
        st.subheader("Policy Checks")
        checks = result.get("policy_checks", [])

        for check in checks:
            status_icon = {"PASS": "✅", "FAIL": "❌", "WARNING": "⚠️"}
            st.markdown(
                f"{status_icon.get(check['status'], '❓')} **{check['check']}**: {check['details']}"
            )

        st.divider()

        exceptions = result.get("exceptions", [])
        if exceptions:
            st.subheader("⚠️ Policy Exceptions")
            for exc in exceptions:
                st.warning(exc)

        st.divider()

        st.subheader("📎 Required Documents")
        docs = result.get("required_documents", [])
        for doc in docs:
            st.write(f"- {doc}")

    with tab4:
        st.subheader("Decision Memo")
        st.markdown(result.get("decision_memo", "No memo available"))

    with tab5:
        st.subheader("Audit Trail")
        audit = result.get("audit_trail", {})
        st.json(audit)

    # Case creation
    st.divider()
    st.header("📁 Case Management")

    col1, col2 = st.columns([3, 1])

    with col1:
        if st.session_state.case_id:
            st.success(f"✅ Case created: **{st.session_state.case_id}**")
        else:
            st.info(
                "Create a case to store this assessment in the case management system"
            )

    with col2:
        if not st.session_state.case_id:
            if st.button("📁 Create Case", type="primary", use_container_width=True):
                try:
                    # Create case via direct API
                    case_data = {
                        "applicant_name": result["normalized_application"][
                            "applicant_name"
                        ],
                        "loan_amount": result["normalized_application"]["loan_amount"],
                        "risk_band": result["risk_band"],
                        "confidence": result["confidence"],
                        "recommended_action": result["recommended_action"],
                        "required_documents": result.get("required_documents", []),
                        "policy_exceptions": result.get("exceptions", []),
                        "notes": result.get("decision_memo", "")[:500],
                    }

                    response = httpx.post(
                        f"{CASE_SERVICE_URL}/cases", json=case_data, timeout=10.0
                    )

                    if response.status_code == 200:
                        case_result = response.json()
                        st.session_state.case_id = case_result["case_id"]
                        st.rerun()
                    else:
                        st.error(f"Failed to create case: {response.text}")

                except Exception as e:
                    st.error(f"Error creating case: {str(e)}")
        else:
            if st.button("🔄 Reset", use_container_width=True):
                st.session_state.case_id = None
                st.session_state.assessment_result = None
                st.rerun()

else:
    st.info(
        "👈 Select a scenario or paste a loan application JSON in the sidebar, then click 'Assess Risk'"
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
