FROM pytorch/pytorch:2.15.0-cuda13.2-cudnn9-runtime
WORKDIR /app
COPY requirements.txt .
RUN python -m venv --without-pip --system-site-packages /opt/embeddings \
    && /opt/embeddings/bin/python -m pip install --no-cache-dir -r requirements.txt
ENV PATH="/opt/embeddings/bin:${PATH}"
COPY server.py .
EXPOSE 8081
CMD ["python", "server.py"]
