FROM python:3.12-slim
WORKDIR /w
COPY requirements.txt .
RUN pip install -r requirements.txt
COPY src/ ./src/
CMD ["python", "-m", "src.main"]
