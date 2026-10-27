# Windows Matriks bridge — Read-only step

Bridge, Ubuntu transport katmanını MatriksIQ'nun resmi API socket'ine bağlar.
MatriksIQ API'si Tiny10 içinde `127.0.0.1:18890` adresinde çalışır; Ubuntu
bridge portu ise `0.0.0.0:9000` olarak kalır. İlk aşamada yalnızca keepalive,
hesap sorgusu ve bağlantı durumu okunur. Emir komutları bilinçli olarak
`execution_disabled` döndürür.

## Tiny10 içinde çalıştırma

`MatriksBridge.cs` dosyasını `C:\bot_emir` içine kopyalayın. Tiny10'da
`dotnet` veya Visual Studio bulunmuyorsa bu dosya tek başına çalıştırılamaz;
önce desteklenen .NET runtime/SDK veya hazır bir self-contained build gerekir.

Derlenmiş bir .NET console projesi oluşturulduktan sonra:

```powershell
dotnet run -- 9000
```

Farklı portlar için ortam değişkenleri kullanılabilir:

```powershell
$env:MATRIKS_API_HOST="127.0.0.1"
$env:MATRIKS_API_PORT="18890"
$env:MATRIKS_BRIDGE_PORT="9000"
dotnet run -- 9000
```

Windows Firewall'da yalnızca Ubuntu VM ağından TCP/9000 erişimine izin verin.
Public internetten port açmayın.

## Ubuntu probe

NDJSON health probe'u:

```bash
MATRIKS_BRIDGE_ADDR=192.168.122.186:9000 \
  bash scripts/matriks/probe_bridge.sh
```

Probe sırasıyla `health`, `session_status` ve `accounts` komutlarını gönderir.
MatriksIQ açık ve API lisansı/bağlantısı etkin değilse ikinci ve üçüncü
komutlarda açık bir `matriks_api_unavailable` hatası beklenir.

MatriksIQ açık ve API erişilebilir olduğunda beklenen health yanıtı:

```json
{"command":"health","response":{"id":"...","ok":true,...}}
```

`session_status` ve `accounts` yanıtları MatriksIQ'nun ham API cevabını
`api_response` veya `accounts` alanında taşır. Bu aşamada hiçbir emir
gönderilmez; `submit_order`, `cancel_order` ve `replace_order` her zaman
`execution_disabled` döndürür.
