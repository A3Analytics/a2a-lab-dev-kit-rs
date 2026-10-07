FROM mcr.microsoft.com/dotnet/sdk:10.0
WORKDIR /src
COPY . .
WORKDIR /src/src/Examples/ServerInitiatedConnection/TestServer.App
RUN dotnet build DeviceServer.App.csproj -c Release
ENTRYPOINT ["dotnet", "run", "-c", "Release", "--no-build", "--project", "DeviceServer.App.csproj"]
