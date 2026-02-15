import os
import gradio as gr

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from typing import Optional
from openai import OpenAI

app = FastAPI()


# Define the request model
class EnergySourceRequest(BaseModel):
    energy_source: str
    consideration: Optional[str] = None


class EnergySourceResponse(BaseModel):
    energy_source: str
    consideration: Optional[str] = None


# Post method for device summary
@app.post("/agent/energy_source_info")
def get_workforce(request: EnergySourceRequest):
    """
    Endpoint to get details about energy source
    """
    considertion = "You don't have any specific consideration. Feel free to talk in a more open ended fashion"

    if request.consideration is not None:
        considertion = f"Add specific focus on the following consideration when you summarize the content for the energy source: {request.consideration}"

    response = {
        "energy_source": request.energy_source,
        "consideration": considertion,
    }
    return response


if __name__ == "__main__":
    app.run(debug=True)
