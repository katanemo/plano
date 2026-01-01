import ast
import sys
import yaml
from typing import Any

FLASK_ROUTE_DECORATORS = ["route", "get", "post", "put", "delete", "patch"]
FASTAPI_ROUTE_DECORATORS = ["get", "post", "put", "delete", "patch"]


def detect_framework(tree: Any) -> str:
    """Detect whether the file is using Flask or FastAPI based on imports."""
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom):
            if node.module == "flask":
                return "flask"
            elif node.module == "fastapi":
                return "fastapi"
    return "unknown"


def get_route_decorators(node: Any, framework: str) -> list:
    """Extract route decorators based on the framework."""
    decorators = []
    for decorator in node.decorator_list:
        if isinstance(decorator, ast.Call) and isinstance(
            decorator.func, ast.Attribute
        ):
            if framework == "flask" and decorator.func.attr in FLASK_ROUTE_DECORATORS:
                decorators.append(decorator.func.attr)
            elif (
                framework == "fastapi"
                and decorator.func.attr in FASTAPI_ROUTE_DECORATORS
            ):
                decorators.append(decorator.func.attr)
    return decorators


def get_route_path(node: Any, framework: str) -> str:
    """Extract route path based on the framework."""
    for decorator in node.decorator_list:
        if isinstance(decorator, ast.Call) and decorator.args:
            return decorator.args[0].s  # Assuming it's a string literal


def is_pydantic_model(annotation: ast.expr, tree: ast.AST) -> bool:
    """Check if a given type annotation is a Pydantic model."""
    # We walk through the AST to find class definitions and check if they inherit from Pydantic's BaseModel
    if isinstance(annotation, ast.Name):
        for node in ast.walk(tree):
            if isinstance(node, ast.ClassDef) and node.name == annotation.id:
                for base in node.bases:
                    if isinstance(base, ast.Name) and base.id == "BaseModel":
                        return True
    return False


def get_pydantic_model_fields(model_name: str, tree: ast.AST) -> list:
    """Extract fields from a Pydantic model, handling list, tuple, set, dict types, and direct default values."""
    fields = []

    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef) and node.name == model_name:
            for stmt in node.body:
                if isinstance(stmt, ast.AnnAssign):
                    # Initialize the default field description
                    field_type = "Unknown: Please Fix This!"
                    description = "Field, description not present. Please fix."
                    default_value = None
                    required = True  # Assume the field is required initially

                    # Check if the field uses Field() with required status and description
                    if (
                        stmt.value
                        and isinstance(stmt.value, ast.Call)
                        and isinstance(stmt.value.func, ast.Name)
                        and stmt.value.func.id == "Field"
                    ):
                        # Extract the description argument inside the Field call
                        for keyword in stmt.value.keywords:
                            if keyword.arg == "description" and isinstance(
                                keyword.value, ast.Str
                            ):
                                description = keyword.value.s
                            if keyword.arg == "default":
                                default_value = keyword.value
                        # If Ellipsis (...) is used, it means the field is required
                        if (
                            stmt.value.args
                            and isinstance(stmt.value.args[0], ast.Constant)
                            and stmt.value.args[0].value is Ellipsis
                        ):
                            required = True
                        else:
                            required = False

                    # Handle direct default values (e.g., name: str = "John Doe")
                    elif stmt.value is not None:
                        if isinstance(stmt.value, ast.Constant):
                            # Set the default value from the assignment (e.g., name: str = "John Doe")
                            default_value = stmt.value.value
                            required = (
                                False  # Not required since it has a default value
                            )

                    # Always extract the field type, even if there's a default value
                    if isinstance(stmt.annotation, ast.Subscript):
                        # Get the base type (list, tuple, set, dict)
                        base_type = (
                            stmt.annotation.value.id
                            if isinstance(stmt.annotation.value, ast.Name)
                            else "Unknown"
                        )

                        # Handle only list, tuple, set, dict and ignore the inner types
                        if base_type.lower() in ["list", "tuple", "set", "dict"]:
                            field_type = base_type.lower()

                    # Handle the ellipsis '...' for required fields if no Field() call
                    elif (
                        isinstance(stmt.value, ast.Constant)
                        and stmt.value.value is Ellipsis
                    ):
                        required = True

                    # Handle simple types like str, int, etc.
                    if isinstance(stmt.annotation, ast.Name):
                        field_type = stmt.annotation.id

                    field_info = {
                        "name": stmt.target.id,
                        "type": field_type,  # Always set the field type
                        "description": description,
                        "default": default_value,  # Handle direct default values
                        "required": required,
                    }
                    fields.append(field_info)

    return fields


def get_function_parameters(node: ast.FunctionDef, tree: ast.AST) -> list:
    """Extract the parameters and their types from the function definition."""
    parameters = []

    # Extract docstring to find descriptions
    docstring = ast.get_docstring(node)
    arg_descriptions = extract_arg_descriptions_from_docstring(docstring)

    # Extract default values
    defaults = [None] * (
        len(node.args.args) - len(node.args.defaults)
    ) + node.args.defaults  # Align defaults with args
    for arg, default in zip(node.args.args, defaults):
        if arg.arg != "self":  # Skip 'self' or 'cls' in class methods
            param_info = {
                "name": arg.arg,
                "description": arg_descriptions.get(arg.arg, "[ADD DESCRIPTION]"),
            }

            # Handle Pydantic model types
            if hasattr(arg, "annotation") and is_pydantic_model(arg.annotation, tree):
                # Extract and flatten Pydantic model fields
                pydantic_fields = get_pydantic_model_fields(arg.annotation.id, tree)
                parameters.extend(
                    pydantic_fields
                )  # Flatten the model fields into the parameters list
                continue  # Skip adding the current param_info for the model since we expand the fields

            # Handle standard Python types (int, float, str, etc.)
            elif hasattr(arg, "annotation") and isinstance(arg.annotation, ast.Name):
                if arg.annotation.id in [
                    "int",
                    "float",
                    "bool",
                    "str",
                    "list",
                    "tuple",
                    "set",
                    "dict",
                ]:
                    param_info["type"] = arg.annotation.id
                else:
                    param_info["type"] = "[UNKNOWN - PLEASE FIX]"

            # Handle generic subscript types (e.g., Optional, List[Type], etc.)
            elif hasattr(arg, "annotation") and isinstance(
                arg.annotation, ast.Subscript
            ):
                if isinstance(
                    arg.annotation.value, ast.Name
                ) and arg.annotation.value.id in ["list", "tuple", "set", "dict"]:
                    param_info[
                        "type"
                    ] = f"{arg.annotation.value.id}"  # e.g., "List", "Tuple", etc.
                else:
                    param_info["type"] = "[UNKNOWN - PLEASE FIX]"

            # Default for unknown types
            else:
                param_info[
                    "type"
                ] = "[UNKNOWN - PLEASE FIX]"  # If unable to detect type

            # Handle default values
            if default is not None:
                if isinstance(default, ast.Constant) or isinstance(
                    default, ast.NameConstant
                ):
                    param_info[
                        "default"
                    ] = default.value  # Use the default value directly
                else:
                    param_info["default"] = "[UNKNOWN DEFAULT]"  # Unknown default type
                param_info["required"] = False  # Optional since it has a default value
            else:
                param_info["default"] = None
                param_info["required"] = True  # Required if no default value

            parameters.append(param_info)

    return parameters


def get_function_docstring(node: Any) -> str:
    """Extract the function's docstring description if present."""
    # Check if the first node is a docstring
    if isinstance(node.body[0], ast.Expr) and isinstance(node.body[0].value, ast.Str):
        # Get the entire docstring
        full_docstring = node.body[0].value.s.strip()

        # Split the docstring by double newlines (to separate description from fields like Args)
        description = full_docstring.split("\n\n")[0].strip()

        return description

    return "No description provided."


def extract_arg_descriptions_from_docstring(docstring: str) -> dict:
    """Extract descriptions for function parameters from the 'Args' section of the docstring."""
    descriptions = {}
    if not docstring:
        return descriptions

    in_args_section = False
    current_param = None
    for line in docstring.splitlines():
        line = line.strip()

        # Detect the start of the 'Args' section
        if line.startswith("Args:"):
            in_args_section = True
            continue  # Proceed to the next line after 'Args:'

        # End of 'Args' section if no indentation and no colon
        if in_args_section and not line.startswith(" ") and ":" not in line:
            break  # Stop processing if we reach a new section

        # Process lines in the 'Args' section
        if in_args_section:
            if ":" in line:
                # Extract parameter name and description
                param_name, description = line.split(":", 1)
                descriptions[param_name.strip()] = description.strip()
                current_param = param_name.strip()
            elif current_param and line.startswith(" "):
                # Handle multiline descriptions (indented lines)
                descriptions[current_param] += f" {line.strip()}"

    return descriptions


def generate_prompt_targets(input_file_path: str) -> None:
    """Introspect routes and generate YAML for either Flask or FastAPI."""
    with open(input_file_path, "r") as source:
        tree = ast.parse(source.read())

    # Detect the framework (Flask or FastAPI)
    framework = detect_framework(tree)
    if framework == "unknown":
        print("Could not detect Flask or FastAPI in the file.")
        return

    # Extract routes
    routes = []
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            route_decorators = get_route_decorators(node, framework)
            if route_decorators:
                route_path = get_route_path(node, framework)
                function_params = get_function_parameters(
                    node, tree
                )  # Get parameters for the route
                function_docstring = get_function_docstring(node)  # Extract docstring
                routes.append(
                    {
                        "name": node.name,
                        "path": route_path,
                        "methods": route_decorators,
                        "parameters": function_params,  # Add parameters to the route
                        "description": function_docstring,  # Add the docstring as the description
                    }
                )

    # Generate YAML structure
    output_structure = {"prompt_targets": []}

    for route in routes:
        target = {
            "name": route["name"],
            "endpoint": [
                {
                    "name": "app_server",
                    "path": route["path"],
                }
            ],
            "description": route["description"],  # Use extracted docstring
            "parameters": [
                {
                    "name": param["name"],
                    "type": param["type"],
                    "description": f"{param['description']}",
                    **(
                        {"default": param["default"]}
                        if "default" in param and param["default"] is not None
                        else {}
                    ),  # Only add default if it's set
                    "required": param["required"],
                }
                for param in route["parameters"]
            ],
        }

        if route["name"] == "default":
            # Special case for `information_extraction` based on your YAML format
            target["type"] = "default"
            target["auto-llm-dispatch-on-response"] = True

        output_structure["prompt_targets"].append(target)

    # Output as YAML
    print(
        yaml.dump(output_structure, sort_keys=False, default_flow_style=False, indent=3)
    )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python targets.py <input_file>")
        sys.exit(1)

    input_file = sys.argv[1]

    # Automatically generate the output file name
    if input_file.endswith(".py"):
        output_file = input_file.replace(".py", "_prompt_targets.yml")
    else:
        print("Error: Input file must be a .py file")
        sys.exit(1)

    # Call the function with the input and generated output file names
    generate_prompt_targets(input_file, output_file)

# Example usage:
# python targets.py api.yaml
