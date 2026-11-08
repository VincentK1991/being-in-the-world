FROM python:3.12-slim
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY server.py worker.py test_worker.py ./
EXPOSE 8082
CMD ["python", "server.py"]
