import click
from fastmcp import FastMCP

mcp = None


@click.command()
@click.option(
    "--transport",
    "transport",
    default="streamable-http",
    help="Transport type: stdio or sse",
)
@click.option("--host", "host", default="localhost", help="Host to bind MCP server to")
@click.option("--port", "port", type=int, default=10500, help="Port for MCP server")
@click.option(
    "--agent",
    "agent",
    required=True,
    help="Agent name: query_rewriter, context_builder, or response_generator",
)
@click.option(
    "--name",
    "agent_name",
    default=None,
    help="Custom MCP server name (defaults to agent type)",
)
@click.option(
    "--rest-server",
    "rest_server",
    is_flag=True,
    help="Start REST server instead of MCP server",
)
@click.option("--rest-port", "rest_port", default=8000, help="Port for REST server")
def main(host, port, agent, transport, agent_name, rest_server, rest_port):
    """Start a RAG agent as an MCP server or REST server."""

    # Map friendly names to agent modules
    agent_map = {
        "input_guards": ("rag_agent.input_guards", "Input Guards Agent"),
        "query_rewriter": ("rag_agent.query_rewriter", "Query Rewriter Agent"),
        "context_builder": ("rag_agent.context_builder", "Context Builder Agent"),
        "response_generator": (
            "rag_agent.rag_agent",
            "Response Generator Agent",
        ),
    }

    if agent not in agent_map:
        print(f"Error: Unknown agent '{agent}'")
        print(f"Available agents: {', '.join(agent_map.keys())}")
        return

    module_name, default_name = agent_map[agent]
    mcp_name = agent_name or default_name

    if rest_server:
        # REST server mode - supported for query_rewriter and response_generator
        if agent == "response_generator":
            print(f"Starting REST server on {host}:{rest_port} for agent: {agent}")
            from rag_agent.rag_agent import start_server

            start_server(host=host, port=rest_port)
            return
        elif agent == "query_rewriter":
            print(f"Starting REST server on {host}:{rest_port} for agent: {agent}")
            from rag_agent.query_rewriter import start_server

            start_server(host=host, port=rest_port)
            return
        else:
            print(f"Error: Agent '{agent}' does not support REST server mode.")
            print(
                f"REST server is only supported for: query_rewriter, response_generator"
            )
            print(f"Remove --rest-server flag to start {agent} as an MCP server.")
            return
    else:
        # Only input_guards, query_rewriter and context_builder support MCP
        if agent not in ["input_guards", "query_rewriter", "context_builder"]:
            print(f"Error: Agent '{agent}' does not support MCP mode.")
            print(
                f"MCP is only supported for: input_guards, query_rewriter, context_builder"
            )
            print(f"Use --rest-server flag to start {agent} as a REST server.")
            return

        global mcp
        mcp = FastMCP(mcp_name, host=host, port=port)

        print(f"Starting MCP server: {mcp_name}")
        print(f"  Agent: {agent}")
        print(f"  Transport: {transport}")
        print(f"  Host: {host}")
        print(f"  Port: {port}")

        # Import the agent module to register its tools
        import importlib

        importlib.import_module(module_name)

        print(f"Agent '{agent}' loaded successfully")
        print(f"MCP server ready on {transport}://{host}:{port}")

        mcp.run(transport=transport)


if __name__ == "__main__":
    main()
