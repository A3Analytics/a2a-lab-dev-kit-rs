FROM mcr.microsoft.com/dotnet/sdk:10.0
WORKDIR /src
COPY sila-csharp /src/sila
COPY sila-provider-driver /src/driver
WORKDIR /src/driver
RUN dotnet build Driver.csproj -c Release
ENTRYPOINT ["dotnet", "run", "-c", "Release", "--no-build", "--project", "Driver.csproj"]
