FROM python:3.12-slim

# The mounted plexi binary links libasound. This base image does not ship it.
RUN apt-get update \
 && apt-get install -y --no-install-recommends libasound2t64 \
 && rm -rf /var/lib/apt/lists/*

COPY runner.py relay.py /opt/house/
COPY static /opt/house/static
COPY agent /opt/house/agent

ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
WORKDIR /opt/house
