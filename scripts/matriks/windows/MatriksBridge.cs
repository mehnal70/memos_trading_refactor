using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;

// MatriksIQ/Ziraat bridge hazırlığı.
// Bu aşama yalnız read-only hesap ve bağlantı sorgularını destekler.
internal static class MatriksBridge
{
    private const int DefaultBridgePort = 9000;
    private const int DefaultMatriksPort = 18890;
    private static readonly MatriksApiClient Matriks = new(
        Environment.GetEnvironmentVariable("MATRIKS_API_HOST") ?? "127.0.0.1",
        ReadPort("MATRIKS_API_PORT", DefaultMatriksPort));

    public static async Task Main(string[] args)
    {
        var port = args.Length > 0
            ? ParsePort(args[0], DefaultBridgePort)
            : ReadPort("MATRIKS_BRIDGE_PORT", DefaultBridgePort);
        var listener = new TcpListener(IPAddress.Any, port);
        listener.Start();
        Console.WriteLine($"Matriks bridge listening on 0.0.0.0:{port}");
        Console.WriteLine($"Matriks API target: {Matriks.Host}:{Matriks.Port}");

        while (true)
        {
            var client = await listener.AcceptTcpClientAsync();
            _ = HandleClientAsync(client);
        }
    }

    private static async Task HandleClientAsync(TcpClient client)
    {
        using (client)
        {
            await using var stream = client.GetStream();
            using var reader = new StreamReader(stream, Encoding.UTF8, leaveOpen: true);
            await using var writer = new StreamWriter(
                stream,
                new UTF8Encoding(false),
                1024,
                leaveOpen: true)
            {
                AutoFlush = true
            };

            while (client.Connected)
            {
                var line = await reader.ReadLineAsync();
                if (line is null)
                    break;

                try
                {
                    using var request = JsonDocument.Parse(line);
                    var root = request.RootElement;
                    var id = root.TryGetProperty("id", out var idElement)
                        ? idElement.GetString() ?? string.Empty
                        : string.Empty;
                    var command = root.TryGetProperty("cmd", out var commandElement)
                        ? commandElement.GetString()?.Trim().ToLowerInvariant()
                        : null;

                    var response = command switch
                    {
                        "health" => Response(id, new
                        {
                            ok = true,
                            service = "matriks-bridge",
                            execution_enabled = false,
                            api_host = Matriks.Host,
                            api_port = Matriks.Port
                        }),
                        "session_status" => await SessionStatusAsync(id),
                        "accounts" => await AccountsAsync(id),
                        "submit_order" or "cancel_order" or "replace_order" =>
                            Error(id, "execution_disabled"),
                        _ => Error(id, "unsupported_command")
                    };

                    await writer.WriteLineAsync(response);
                }
                catch (JsonException)
                {
                    await writer.WriteLineAsync(Error(string.Empty, "invalid_json"));
                }
                catch (SocketException exception)
                {
                    await writer.WriteLineAsync(Error(string.Empty, $"matriks_api_socket_error:{exception.SocketErrorCode}"));
                }
                catch (IOException exception)
                {
                    await writer.WriteLineAsync(Error(string.Empty, $"matriks_api_io_error:{exception.Message}"));
                }
            }
        }
    }

    private static async Task<string> SessionStatusAsync(string id)
    {
        try
        {
            var response = await Matriks.SendAsync(new { ApiCommands = 6 });
            return Response(id, new
            {
                ok = true,
                matriks_connected = true,
                ziraat_connected = true,
                read_only = true,
                execution_enabled = false,
                api_response = response.RootElement
            });
        }
        catch (SocketException exception)
        {
            return Error(id, $"matriks_api_unavailable:{exception.SocketErrorCode}");
        }
        catch (IOException exception)
        {
            return Error(id, $"matriks_api_io_error:{exception.Message}");
        }
    }

    private static async Task<string> AccountsAsync(string id)
    {
        try
        {
            var response = await Matriks.SendAsync(new { ApiCommands = 0 });
            return Response(id, new
            {
                ok = true,
                read_only = true,
                execution_enabled = false,
                accounts = response.RootElement
            });
        }
        catch (SocketException exception)
        {
            return Error(id, $"matriks_api_unavailable:{exception.SocketErrorCode}");
        }
        catch (IOException exception)
        {
            return Error(id, $"matriks_api_io_error:{exception.Message}");
        }
    }

    private static string Response(string id, object payload)
    {
        var result = new Dictionary<string, object?>
        {
            ["id"] = id
        };
        foreach (var property in payload.GetType().GetProperties())
            result[property.Name] = property.GetValue(payload);
        return JsonSerializer.Serialize(result);
    }

    private static string Error(string id, string error) =>
        JsonSerializer.Serialize(new { id, ok = false, error });

    private static int ReadPort(string variable, int fallback)
    {
        var raw = Environment.GetEnvironmentVariable(variable);
        return raw is null ? fallback : ParsePort(raw, fallback);
    }

    private static int ParsePort(string raw, int fallback) =>
        int.TryParse(raw, out var port) && port is > 0 and <= 65535
            ? port
            : fallback;
}

internal sealed class MatriksApiClient
{
    private const byte MessageTerminator = 11;
    private readonly SemaphoreSlim gate = new(1, 1);
    private readonly string host;
    private readonly int port;
    private TcpClient? client;
    private NetworkStream? stream;

    public MatriksApiClient(string host, int port)
    {
        this.host = host;
        this.port = port;
    }

    public string Host => host;
    public int Port => port;

    public async Task<JsonDocument> SendAsync(object request)
    {
        await gate.WaitAsync();
        try
        {
            await EnsureConnectedAsync();
            var payload = JsonSerializer.SerializeToUtf8Bytes(request);
            await stream!.WriteAsync(payload);
            await stream.WriteAsync(new[] { MessageTerminator });
            await stream.FlushAsync();
            return await ReadJsonAsync(stream);
        }
        finally
        {
            gate.Release();
        }
    }

    private async Task EnsureConnectedAsync()
    {
        if (client?.Connected == true && stream is not null)
            return;

        client?.Dispose();
        client = new TcpClient();
        await client.ConnectAsync(host, port);
        stream = client.GetStream();
        await WriteFramedAsync("SetMessageType0");
        await WriteFramedAsync(JsonSerializer.Serialize(new
        {
            ApiCommands = 11,
            BroadcastMode = 1
        }));
        _ = await ReadJsonAsync(stream);
    }

    private async Task WriteFramedAsync(string payload)
    {
        var bytes = Encoding.UTF8.GetBytes(payload);
        await stream!.WriteAsync(bytes);
        await stream.WriteAsync(new[] { MessageTerminator });
        await stream.FlushAsync();
    }

    private static async Task<JsonDocument> ReadJsonAsync(NetworkStream networkStream)
    {
        using var buffer = new MemoryStream();
        var chunk = new byte[4096];
        while (true)
        {
            var count = await networkStream.ReadAsync(chunk);
            if (count == 0)
                throw new IOException("Matriks API connection closed");

            var terminatorIndex = Array.IndexOf(chunk, MessageTerminator, 0, count);
            if (terminatorIndex >= 0)
            {
                buffer.Write(chunk, 0, terminatorIndex);
                return JsonDocument.Parse(buffer.ToArray());
            }

            buffer.Write(chunk, 0, count);
            if (buffer.Length > 1_048_576)
                throw new IOException("Matriks API response exceeds 1 MiB");
        }
    }
}
