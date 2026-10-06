FROM python:3.12-slim
RUN pip install --no-cache-dir sila2-interop-communication-tester==0.10.3
ENTRYPOINT ["python", "-m", "sila2_interop_communication_tester.test_client"]
