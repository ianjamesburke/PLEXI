FROM python:3.12-slim

WORKDIR /app
# Build from the repository root:
#   docker build -f services/relay/Dockerfile .
COPY services/relay/relay.py /app/services/relay/relay.py
COPY clients/phone-web/static /app/clients/phone-web/static

ENV PORT=8080
EXPOSE 8080

# In-memory only. The container has no volume for message bodies.
# relay.py reads PORT when --port is omitted.
CMD ["python3", "-u", "/app/services/relay/relay.py", "--host", "0.0.0.0"]
