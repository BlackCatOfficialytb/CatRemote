from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
import json
import os

app = FastAPI(title="CatRemote Local API")

CONFIG_FILE = "config.json"

class ConfigData(BaseModel):
    host_code: str
    preferred_protocol: str
    
@app.get("/api/config")
def read_config():
    if not os.path.exists(CONFIG_FILE):
        return {"host_code": "123456", "preferred_protocol": "WebRTC"}
    
    try:
        with open(CONFIG_FILE, "r") as f:
            return json.load(f)
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/api/config")
def write_config(data: ConfigData):
    try:
        with open(CONFIG_FILE, "w") as f:
            json.dump(data.model_dump(), f, indent=4)
        return {"status": "success"}
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=8000)
