// Kast Virtual Display: controlador de pantallas virtuales (Indirect Display, IddCx) para Kast Link.
//
// Basado en el ejemplo IddSampleDriver de Microsoft (Windows-driver-samples, licencia MS-PL; ver ../LICENSE.txt).
// Copyright (c) Microsoft Corporation. Cambios: pantallas a petición de la aplicación (IOCTLs de ../include/kastdisplay.h)
// con el tamaño y la frecuencia exactos, EDID propio, vigilancia de la aplicación y nombres de Kast.

#include "Driver.h"
#include "Edid.h"

#include <algorithm>

using namespace Kast;
using namespace Microsoft::WRL;

static const GUID KastInterface = KAST_DISPLAY_INTERFACE;

#pragma region Modos

static std::mutex g_ModesLock;
static std::map<ULONG, Mode> g_Modes;

void Kast::RegisterModes(ULONG Serial, const Mode& Wanted)
{
    std::lock_guard<std::mutex> g(g_ModesLock);
    g_Modes[Serial] = Wanted;
}

void Kast::UnregisterModes(ULONG Serial)
{
    std::lock_guard<std::mutex> g(g_ModesLock);
    g_Modes.erase(Serial);
}

bool Kast::ModesForSerial(ULONG Serial, Mode* Wanted)
{
    std::lock_guard<std::mutex> g(g_ModesLock);
    auto it = g_Modes.find(Serial);
    if (it == g_Modes.end()) return false;
    *Wanted = it->second;
    return true;
}

std::vector<Mode> Kast::ModeList(const Mode& Wanted)
{
    static const Mode common[] = {
        { 3840, 2160, 60 }, { 2560, 1600, 60 }, { 2560, 1440, 60 }, { 1920, 1200, 60 }, { 1920, 1080, 60 },
        { 1680, 1050, 60 }, { 1600, 900, 60 }, { 1440, 900, 60 }, { 1366, 768, 60 }, { 1280, 800, 60 },
        { 1280, 720, 60 }, { 1024, 768, 60 },
    };
    std::vector<Mode> modes = { Wanted };
    if (Wanted.Hz != 60) modes.push_back({ Wanted.Width, Wanted.Height, 60 });
    for (const Mode& m : common)
    {
        if (m.Width > Wanted.Width || m.Height > Wanted.Height) continue;
        if (m.Width == Wanted.Width && m.Height == Wanted.Height) continue;
        modes.push_back(m);
    }
    return modes;
}

static void FillSignalInfo(DISPLAYCONFIG_VIDEO_SIGNAL_INFO& Mode, DWORD Width, DWORD Height, DWORD VSync, bool bMonitorMode)
{
    Mode.totalSize.cx = Mode.activeSize.cx = Width;
    Mode.totalSize.cy = Mode.activeSize.cy = Height;

    // Ver https://docs.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_video_signal_info
    Mode.AdditionalSignalInfo.vSyncFreqDivider = bMonitorMode ? 0 : 1;
    Mode.AdditionalSignalInfo.videoStandard = 255;

    Mode.vSyncFreq.Numerator = VSync;
    Mode.vSyncFreq.Denominator = 1;
    Mode.hSyncFreq.Numerator = VSync * Height;
    Mode.hSyncFreq.Denominator = 1;

    Mode.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE;

    Mode.pixelRate = ((UINT64)VSync) * ((UINT64)Width) * ((UINT64)Height);
}

static IDDCX_MONITOR_MODE CreateIddCxMonitorMode(const Mode& m, IDDCX_MONITOR_MODE_ORIGIN Origin)
{
    IDDCX_MONITOR_MODE Mode = {};
    Mode.Size = sizeof(Mode);
    Mode.Origin = Origin;
    FillSignalInfo(Mode.MonitorVideoSignalInfo, m.Width, m.Height, m.Hz, true);
    return Mode;
}

static IDDCX_TARGET_MODE CreateIddCxTargetMode(const Mode& m)
{
    IDDCX_TARGET_MODE Mode = {};
    Mode.Size = sizeof(Mode);
    FillSignalInfo(Mode.TargetVideoSignalInfo.targetVideoSignalInfo, m.Width, m.Height, m.Hz, false);
    return Mode;
}

#pragma endregion

extern "C" DRIVER_INITIALIZE DriverEntry;

EVT_WDF_DRIVER_DEVICE_ADD KastDeviceAdd;
EVT_WDF_DEVICE_D0_ENTRY KastDeviceD0Entry;

EVT_IDD_CX_DEVICE_IO_CONTROL KastIoDeviceControl;
EVT_IDD_CX_ADAPTER_INIT_FINISHED KastAdapterInitFinished;
EVT_IDD_CX_ADAPTER_COMMIT_MODES KastAdapterCommitModes;

EVT_IDD_CX_PARSE_MONITOR_DESCRIPTION KastParseMonitorDescription;
EVT_IDD_CX_MONITOR_GET_DEFAULT_DESCRIPTION_MODES KastMonitorGetDefaultModes;
EVT_IDD_CX_MONITOR_QUERY_TARGET_MODES KastMonitorQueryModes;

EVT_IDD_CX_MONITOR_ASSIGN_SWAPCHAIN KastMonitorAssignSwapChain;
EVT_IDD_CX_MONITOR_UNASSIGN_SWAPCHAIN KastMonitorUnassignSwapChain;

struct DeviceContextWrapper
{
    DeviceContext* pContext;

    void Cleanup()
    {
        delete pContext;
        pContext = nullptr;
    }
};

struct MonitorContextWrapper
{
    MonitorContext* pContext;

    void Cleanup()
    {
        delete pContext;
        pContext = nullptr;
    }
};

WDF_DECLARE_CONTEXT_TYPE(DeviceContextWrapper);
WDF_DECLARE_CONTEXT_TYPE(MonitorContextWrapper);

extern "C" BOOL WINAPI DllMain(_In_ HINSTANCE hInstance, _In_ UINT dwReason, _In_opt_ LPVOID lpReserved)
{
    UNREFERENCED_PARAMETER(hInstance);
    UNREFERENCED_PARAMETER(lpReserved);
    UNREFERENCED_PARAMETER(dwReason);
    return TRUE;
}

_Use_decl_annotations_
extern "C" NTSTATUS DriverEntry(PDRIVER_OBJECT pDriverObject, PUNICODE_STRING pRegistryPath)
{
    WDF_DRIVER_CONFIG Config;
    WDF_OBJECT_ATTRIBUTES Attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&Attributes);
    WDF_DRIVER_CONFIG_INIT(&Config, KastDeviceAdd);
    return WdfDriverCreate(pDriverObject, pRegistryPath, &Attributes, &Config, WDF_NO_HANDLE);
}

_Use_decl_annotations_
NTSTATUS KastDeviceAdd(WDFDRIVER Driver, PWDFDEVICE_INIT pDeviceInit)
{
    UNREFERENCED_PARAMETER(Driver);

    WDF_PNPPOWER_EVENT_CALLBACKS PnpPowerCallbacks;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&PnpPowerCallbacks);
    PnpPowerCallbacks.EvtDeviceD0Entry = KastDeviceD0Entry;
    WdfDeviceInitSetPnpPowerEventCallbacks(pDeviceInit, &PnpPowerCallbacks);

    IDD_CX_CLIENT_CONFIG IddConfig;
    IDD_CX_CLIENT_CONFIG_INIT(&IddConfig);

    // IddCx lleva las peticiones IoDeviceControl a una cola suya: las de la aplicación llegan por aquí.
    IddConfig.EvtIddCxDeviceIoControl = KastIoDeviceControl;
    IddConfig.EvtIddCxAdapterInitFinished = KastAdapterInitFinished;
    IddConfig.EvtIddCxParseMonitorDescription = KastParseMonitorDescription;
    IddConfig.EvtIddCxMonitorGetDefaultDescriptionModes = KastMonitorGetDefaultModes;
    IddConfig.EvtIddCxMonitorQueryTargetModes = KastMonitorQueryModes;
    IddConfig.EvtIddCxAdapterCommitModes = KastAdapterCommitModes;
    IddConfig.EvtIddCxMonitorAssignSwapChain = KastMonitorAssignSwapChain;
    IddConfig.EvtIddCxMonitorUnassignSwapChain = KastMonitorUnassignSwapChain;

    NTSTATUS Status = IddCxDeviceInitConfig(pDeviceInit, &IddConfig);
    if (!NT_SUCCESS(Status)) return Status;

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, DeviceContextWrapper);
    Attr.EvtCleanupCallback = [](WDFOBJECT Object)
    {
        auto* pContext = WdfObjectGet_DeviceContextWrapper(Object);
        if (pContext) pContext->Cleanup();
    };

    WDFDEVICE Device = nullptr;
    Status = WdfDeviceCreate(&pDeviceInit, &Attr, &Device);
    if (!NT_SUCCESS(Status)) return Status;

    Status = IddCxDeviceInitialize(Device);
    if (!NT_SUCCESS(Status)) return Status;

    // La interfaz por la que la aplicación encuentra el adaptador.
    Status = WdfDeviceCreateDeviceInterface(Device, &KastInterface, nullptr);
    if (!NT_SUCCESS(Status)) return Status;

    auto* pContext = WdfObjectGet_DeviceContextWrapper(Device);
    pContext->pContext = new DeviceContext(Device);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KastDeviceD0Entry(WDFDEVICE Device, WDF_POWER_DEVICE_STATE PreviousState)
{
    UNREFERENCED_PARAMETER(PreviousState);
    auto* pContext = WdfObjectGet_DeviceContextWrapper(Device);
    pContext->pContext->InitAdapter();
    return STATUS_SUCCESS;
}

#pragma region Direct3DDevice

Direct3DDevice::Direct3DDevice(LUID AdapterLuid) : AdapterLuid(AdapterLuid)
{
}

HRESULT Direct3DDevice::Init()
{
    HRESULT hr = CreateDXGIFactory2(0, IID_PPV_ARGS(&DxgiFactory));
    if (FAILED(hr)) return hr;

    hr = DxgiFactory->EnumAdapterByLuid(AdapterLuid, IID_PPV_ARGS(&Adapter));
    if (FAILED(hr)) return hr;

    // Con BGRA (lo pide la certificación de Windows).
    return D3D11CreateDevice(Adapter.Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0,
                             D3D11_SDK_VERSION, &Device, nullptr, &DeviceContext);
}

#pragma endregion

#pragma region SwapChainProcessor

SwapChainProcessor::SwapChainProcessor(IDDCX_SWAPCHAIN hSwapChain, std::shared_ptr<Direct3DDevice> Device, HANDLE NewFrameEvent)
    : m_hSwapChain(hSwapChain), m_Device(Device), m_hAvailableBufferEvent(NewFrameEvent)
{
    m_hTerminateEvent.Attach(CreateEvent(nullptr, FALSE, FALSE, nullptr));
    m_hThread.Attach(CreateThread(nullptr, 0, RunThread, this, 0, nullptr));
}

SwapChainProcessor::~SwapChainProcessor()
{
    SetEvent(m_hTerminateEvent.Get());
    if (m_hThread.Get()) WaitForSingleObject(m_hThread.Get(), INFINITE);
}

DWORD CALLBACK SwapChainProcessor::RunThread(LPVOID Argument)
{
    reinterpret_cast<SwapChainProcessor*>(Argument)->Run();
    return 0;
}

void SwapChainProcessor::Run()
{
    // Prioridad multimedia: los fotogramas se devuelven a tiempo aunque el equipo vaya cargado.
    DWORD AvTask = 0;
    HANDLE AvTaskHandle = AvSetMmThreadCharacteristicsW(L"Distribution", &AvTask);

    RunCore();

    // Al terminar se borra la cadena de intercambio, así el sistema da otra si hace falta.
    WdfObjectDelete((WDFOBJECT)m_hSwapChain);
    m_hSwapChain = nullptr;

    AvRevertMmThreadCharacteristics(AvTaskHandle);
}

void SwapChainProcessor::RunCore()
{
    ComPtr<IDXGIDevice> DxgiDevice;
    HRESULT hr = m_Device->Device.As(&DxgiDevice);
    if (FAILED(hr)) return;

    IDARG_IN_SWAPCHAINSETDEVICE SetDevice = {};
    SetDevice.pDevice = DxgiDevice.Get();
    hr = IddCxSwapChainSetDevice(m_hSwapChain, &SetDevice);
    if (FAILED(hr)) return;

    for (;;)
    {
        ComPtr<IDXGIResource> AcquiredBuffer;
        IDARG_OUT_RELEASEANDACQUIREBUFFER Buffer = {};
        hr = IddCxSwapChainReleaseAndAcquireBuffer(m_hSwapChain, &Buffer);

        if (hr == E_PENDING)
        {
            HANDLE WaitHandles[] = { m_hAvailableBufferEvent, m_hTerminateEvent.Get() };
            DWORD WaitResult = WaitForMultipleObjects(ARRAYSIZE(WaitHandles), WaitHandles, FALSE, 16);
            if (WaitResult == WAIT_OBJECT_0 || WaitResult == WAIT_TIMEOUT) continue;
            break;
        }
        if (FAILED(hr)) break; // la cadena se abandonó (p. ej. DXGI_ERROR_ACCESS_LOST)

        // Kast captura la pantalla con Desktop Duplication, como cualquier monitor: aquí sólo se devuelve el fotograma.
        AcquiredBuffer.Attach(Buffer.MetaData.pSurface);
        AcquiredBuffer.Reset();
        hr = IddCxSwapChainFinishedProcessingFrame(m_hSwapChain);
        if (FAILED(hr)) break;
    }
}

#pragma endregion

#pragma region DeviceContext

DeviceContext::DeviceContext(_In_ WDFDEVICE WdfDevice) : m_WdfDevice(WdfDevice)
{
    m_hStop.Attach(CreateEvent(nullptr, TRUE, FALSE, nullptr));
}

DeviceContext::~DeviceContext()
{
    SetEvent(m_hStop.Get());
    if (m_hWatchdog.Get()) WaitForSingleObject(m_hWatchdog.Get(), INFINITE);
}

void DeviceContext::InitAdapter()
{
    IDDCX_ADAPTER_CAPS AdapterCaps = {};
    AdapterCaps.Size = sizeof(AdapterCaps);
    AdapterCaps.MaxMonitorsSupported = KAST_MAX_MONITORS;
    AdapterCaps.EndPointDiagnostics.Size = sizeof(AdapterCaps.EndPointDiagnostics);
    AdapterCaps.EndPointDiagnostics.GammaSupport = IDDCX_FEATURE_IMPLEMENTATION_NONE;
    AdapterCaps.EndPointDiagnostics.TransmissionType = IDDCX_TRANSMISSION_TYPE_WIRED_OTHER;
    AdapterCaps.EndPointDiagnostics.pEndPointFriendlyName = L"Kast Virtual Display";
    AdapterCaps.EndPointDiagnostics.pEndPointManufacturerName = L"Kast";
    AdapterCaps.EndPointDiagnostics.pEndPointModelName = L"Kast Virtual Display";

    IDDCX_ENDPOINT_VERSION Version = {};
    Version.Size = sizeof(Version);
    Version.MajorVer = 1;
    AdapterCaps.EndPointDiagnostics.pFirmwareVersion = &Version;
    AdapterCaps.EndPointDiagnostics.pHardwareVersion = &Version;

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, DeviceContextWrapper);

    IDARG_IN_ADAPTER_INIT AdapterInit = {};
    AdapterInit.WdfDevice = m_WdfDevice;
    AdapterInit.pCaps = &AdapterCaps;
    AdapterInit.ObjectAttributes = &Attr;

    IDARG_OUT_ADAPTER_INIT AdapterInitOut;
    NTSTATUS Status = IddCxAdapterInitAsync(&AdapterInit, &AdapterInitOut);
    if (NT_SUCCESS(Status))
    {
        m_Adapter = AdapterInitOut.AdapterObject;
        // El adaptador apunta a este contexto (sin borrarlo: es del dispositivo).
        auto* pContext = WdfObjectGet_DeviceContextWrapper(AdapterInitOut.AdapterObject);
        pContext->pContext = this;
    }
}

void DeviceContext::AdapterReady()
{
    std::lock_guard<std::mutex> g(m_Lock);
    m_Ready = true;
    if (!m_hWatchdog.Get()) m_hWatchdog.Attach(CreateThread(nullptr, 0, WatchdogThread, this, 0, nullptr));
}

void DeviceContext::Touch()
{
    std::lock_guard<std::mutex> g(m_Lock);
    m_LastTouch = GetTickCount64();
}

ULONG DeviceContext::MonitorCount()
{
    std::lock_guard<std::mutex> g(m_Lock);
    return (ULONG)m_Slots.size();
}

NTSTATUS DeviceContext::AddMonitor(const Mode& Wanted, ULONG* Id)
{
    std::lock_guard<std::mutex> g(m_Lock);
    if (!m_Ready) return STATUS_DEVICE_NOT_READY;
    if (m_Slots.size() >= KAST_MAX_MONITORS) return STATUS_INSUFFICIENT_RESOURCES;

    // El primer conector libre.
    UINT Connector = 0;
    while (std::any_of(m_Slots.begin(), m_Slots.end(), [&](const Slot& s) { return s.Connector == Connector; })) Connector++;
    const ULONG Serial = m_NextId++;

    BYTE Edid[EdidSize];
    BuildEdid(Edid, Wanted.Width, Wanted.Height, Wanted.Hz, Serial);
    RegisterModes(Serial, Wanted);

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, MonitorContextWrapper);
    Attr.EvtCleanupCallback = [](WDFOBJECT Object)
    {
        auto* pContext = WdfObjectGet_MonitorContextWrapper(Object);
        if (pContext) pContext->Cleanup();
    };

    IDDCX_MONITOR_INFO MonitorInfo = {};
    MonitorInfo.Size = sizeof(MonitorInfo);
    MonitorInfo.MonitorType = DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI;
    MonitorInfo.ConnectorIndex = Connector;
    MonitorInfo.MonitorDescription.Size = sizeof(MonitorInfo.MonitorDescription);
    MonitorInfo.MonitorDescription.Type = IDDCX_MONITOR_DESCRIPTION_TYPE_EDID;
    MonitorInfo.MonitorDescription.DataSize = EdidSize;
    MonitorInfo.MonitorDescription.pData = Edid;
    // Cada pantalla es un monitor distinto (no forma parte del adaptador).
    CoCreateGuid(&MonitorInfo.MonitorContainerId);

    IDARG_IN_MONITORCREATE MonitorCreate = {};
    MonitorCreate.ObjectAttributes = &Attr;
    MonitorCreate.pMonitorInfo = &MonitorInfo;

    IDARG_OUT_MONITORCREATE MonitorCreateOut;
    NTSTATUS Status = IddCxMonitorCreate(m_Adapter, &MonitorCreate, &MonitorCreateOut);
    if (!NT_SUCCESS(Status))
    {
        UnregisterModes(Serial);
        return Status;
    }
    auto* pMonitorContextWrapper = WdfObjectGet_MonitorContextWrapper(MonitorCreateOut.MonitorObject);
    pMonitorContextWrapper->pContext = new MonitorContext(MonitorCreateOut.MonitorObject, Wanted);

    // Se enchufa: Windows lee su EDID, le da modos y la suma al escritorio.
    IDARG_OUT_MONITORARRIVAL ArrivalOut;
    Status = IddCxMonitorArrival(MonitorCreateOut.MonitorObject, &ArrivalOut);
    if (!NT_SUCCESS(Status))
    {
        WdfObjectDelete((WDFOBJECT)MonitorCreateOut.MonitorObject);
        UnregisterModes(Serial);
        return Status;
    }
    m_Slots.push_back({ MonitorCreateOut.MonitorObject, Serial, Connector });
    m_LastTouch = GetTickCount64();
    *Id = Serial;
    return STATUS_SUCCESS;
}

NTSTATUS DeviceContext::RemoveMonitor(ULONG Id)
{
    std::lock_guard<std::mutex> g(m_Lock);
    auto it = std::find_if(m_Slots.begin(), m_Slots.end(), [&](const Slot& s) { return s.Id == Id; });
    if (it == m_Slots.end()) return STATUS_NOT_FOUND;
    // Se desenchufa: Windows lleva sus ventanas a las pantallas que quedan.
    NTSTATUS Status = IddCxMonitorDeparture(it->Monitor);
    UnregisterModes(it->Id);
    m_Slots.erase(it);
    return Status;
}

void DeviceContext::RemoveAll()
{
    std::lock_guard<std::mutex> g(m_Lock);
    for (const Slot& s : m_Slots)
    {
        IddCxMonitorDeparture(s.Monitor);
        UnregisterModes(s.Id);
    }
    m_Slots.clear();
}

DWORD CALLBACK DeviceContext::WatchdogThread(LPVOID Argument)
{
    reinterpret_cast<DeviceContext*>(Argument)->Watchdog();
    return 0;
}

// Si la aplicación deja de hablar con el controlador (se cerró o se colgó), sus pantallas se quitan.
void DeviceContext::Watchdog()
{
    while (WaitForSingleObject(m_hStop.Get(), 250) == WAIT_TIMEOUT)
    {
        bool expired;
        {
            std::lock_guard<std::mutex> g(m_Lock);
            expired = !m_Slots.empty() && GetTickCount64() - m_LastTouch > KAST_WATCHDOG_MS;
        }
        if (expired) RemoveAll();
    }
}

#pragma endregion

#pragma region MonitorContext

MonitorContext::MonitorContext(_In_ IDDCX_MONITOR Monitor, const Mode& Wanted) : Wanted(Wanted), m_Monitor(Monitor)
{
}

MonitorContext::~MonitorContext()
{
    m_ProcessingThread.reset();
}

void MonitorContext::AssignSwapChain(IDDCX_SWAPCHAIN SwapChain, LUID RenderAdapter, HANDLE NewFrameEvent)
{
    m_ProcessingThread.reset();
    auto Device = std::make_shared<Direct3DDevice>(RenderAdapter);
    if (FAILED(Device->Init()))
    {
        // Sin Direct3D se borra la cadena: el sistema lo vuelve a intentar con otra.
        WdfObjectDelete(SwapChain);
    }
    else
    {
        m_ProcessingThread.reset(new SwapChainProcessor(SwapChain, Device, NewFrameEvent));
    }
}

void MonitorContext::UnassignSwapChain()
{
    m_ProcessingThread.reset();
}

#pragma endregion

#pragma region Llamadas de IddCx

_Use_decl_annotations_
VOID KastIoDeviceControl(WDFDEVICE Device, WDFREQUEST Request, size_t OutputBufferLength, size_t InputBufferLength, ULONG IoControlCode)
{
    UNREFERENCED_PARAMETER(OutputBufferLength);
    UNREFERENCED_PARAMETER(InputBufferLength);

    DeviceContext* ctx = WdfObjectGet_DeviceContextWrapper(Device)->pContext;
    NTSTATUS Status = STATUS_INVALID_DEVICE_REQUEST;
    size_t Info = 0;
    if (ctx) ctx->Touch();

    switch (IoControlCode)
    {
    case IOCTL_KAST_INFO:
    {
        KAST_INFO* Out = nullptr;
        Status = WdfRequestRetrieveOutputBuffer(Request, sizeof(KAST_INFO), (PVOID*)&Out, nullptr);
        if (NT_SUCCESS(Status))
        {
            Out->Protocol = KAST_PROTOCOL;
            Out->MaxMonitors = KAST_MAX_MONITORS;
            Out->Monitors = ctx ? ctx->MonitorCount() : 0;
            Out->WatchdogMs = KAST_WATCHDOG_MS;
            Info = sizeof(KAST_INFO);
        }
        break;
    }
    case IOCTL_KAST_ADD:
    {
        KAST_ADD* In = nullptr;
        KAST_ADDED* Out = nullptr;
        Status = WdfRequestRetrieveInputBuffer(Request, sizeof(KAST_ADD), (PVOID*)&In, nullptr);
        if (NT_SUCCESS(Status)) Status = WdfRequestRetrieveOutputBuffer(Request, sizeof(KAST_ADDED), (PVOID*)&Out, nullptr);
        if (!NT_SUCCESS(Status)) break;
        const Mode Wanted = { In->Width, In->Height, In->RefreshHz ? In->RefreshHz : 60 };
        if (!ctx || Wanted.Width < 640 || Wanted.Height < 480 || Wanted.Width > 16384 || Wanted.Height > 16384 || Wanted.Hz < 24 ||
            Wanted.Hz > 240)
        {
            Status = STATUS_INVALID_PARAMETER;
            break;
        }
        Status = ctx->AddMonitor(Wanted, &Out->Id);
        if (NT_SUCCESS(Status)) Info = sizeof(KAST_ADDED);
        break;
    }
    case IOCTL_KAST_REMOVE:
    {
        KAST_REMOVE* In = nullptr;
        Status = WdfRequestRetrieveInputBuffer(Request, sizeof(KAST_REMOVE), (PVOID*)&In, nullptr);
        if (NT_SUCCESS(Status)) Status = ctx ? ctx->RemoveMonitor(In->Id) : STATUS_DEVICE_NOT_READY;
        break;
    }
    case IOCTL_KAST_PING:
        Status = STATUS_SUCCESS;
        break;
    }
    WdfRequestCompleteWithInformation(Request, Status, Info);
}

_Use_decl_annotations_
NTSTATUS KastAdapterInitFinished(IDDCX_ADAPTER AdapterObject, const IDARG_IN_ADAPTER_INIT_FINISHED* pInArgs)
{
    // Listo para pantallas: sin ninguna hasta que la aplicación las pida.
    auto* pDeviceContextWrapper = WdfObjectGet_DeviceContextWrapper(AdapterObject);
    if (NT_SUCCESS(pInArgs->AdapterInitStatus)) pDeviceContextWrapper->pContext->AdapterReady();
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KastAdapterCommitModes(IDDCX_ADAPTER AdapterObject, const IDARG_IN_COMMITMODES* pInArgs)
{
    // No hay hardware que reconfigurar: IddCx se ocupa de la cadena de intercambio.
    UNREFERENCED_PARAMETER(AdapterObject);
    UNREFERENCED_PARAMETER(pInArgs);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KastParseMonitorDescription(const IDARG_IN_PARSEMONITORDESCRIPTION* pInArgs, IDARG_OUT_PARSEMONITORDESCRIPTION* pOutArgs)
{
    // El EDID es nuestro: sus modos son los pedidos para esa pantalla (por su número de serie).
    Mode Wanted;
    if (pInArgs->MonitorDescription.DataSize != EdidSize ||
        !ModesForSerial(EdidSerial((const BYTE*)pInArgs->MonitorDescription.pData), &Wanted))
    {
        return STATUS_INVALID_PARAMETER;
    }
    const std::vector<Mode> Modes = ModeList(Wanted);
    pOutArgs->MonitorModeBufferOutputCount = (UINT)Modes.size();
    if (pInArgs->MonitorModeBufferInputCount < Modes.size())
    {
        // Sin espacio: sólo preguntaba cuántos hay.
        return (pInArgs->MonitorModeBufferInputCount > 0) ? STATUS_BUFFER_TOO_SMALL : STATUS_SUCCESS;
    }
    for (size_t i = 0; i < Modes.size(); i++)
    {
        pInArgs->pMonitorModes[i] = CreateIddCxMonitorMode(Modes[i], IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR);
    }
    pOutArgs->PreferredMonitorModeIdx = 0;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KastMonitorGetDefaultModes(IDDCX_MONITOR MonitorObject, const IDARG_IN_GETDEFAULTDESCRIPTIONMODES* pInArgs,
                                    IDARG_OUT_GETDEFAULTDESCRIPTIONMODES* pOutArgs)
{
    // Sólo para monitores sin EDID: los de Kast siempre lo llevan, pero por si acaso, los modos pedidos.
    auto* pMonitorContextWrapper = WdfObjectGet_MonitorContextWrapper(MonitorObject);
    const Mode Wanted = pMonitorContextWrapper->pContext ? pMonitorContextWrapper->pContext->Wanted : Mode{ 1920, 1080, 60 };
    const std::vector<Mode> Modes = ModeList(Wanted);
    pOutArgs->DefaultMonitorModeBufferOutputCount = (UINT)Modes.size();
    if (pInArgs->DefaultMonitorModeBufferInputCount >= Modes.size())
    {
        for (size_t i = 0; i < Modes.size(); i++)
        {
            pInArgs->pDefaultMonitorModes[i] = CreateIddCxMonitorMode(Modes[i], IDDCX_MONITOR_MODE_ORIGIN_DRIVER);
        }
        pOutArgs->PreferredMonitorModeIdx = 0;
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KastMonitorQueryModes(IDDCX_MONITOR MonitorObject, const IDARG_IN_QUERYTARGETMODES* pInArgs, IDARG_OUT_QUERYTARGETMODES* pOutArgs)
{
    // Los modos que el adaptador puede dar en esta pantalla: los mismos que su monitor.
    auto* pMonitorContextWrapper = WdfObjectGet_MonitorContextWrapper(MonitorObject);
    const Mode Wanted = pMonitorContextWrapper->pContext ? pMonitorContextWrapper->pContext->Wanted : Mode{ 1920, 1080, 60 };
    std::vector<IDDCX_TARGET_MODE> TargetModes;
    for (const Mode& m : ModeList(Wanted)) TargetModes.push_back(CreateIddCxTargetMode(m));

    pOutArgs->TargetModeBufferOutputCount = (UINT)TargetModes.size();
    if (pInArgs->TargetModeBufferInputCount >= TargetModes.size())
    {
        std::copy(TargetModes.begin(), TargetModes.end(), pInArgs->pTargetModes);
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KastMonitorAssignSwapChain(IDDCX_MONITOR MonitorObject, const IDARG_IN_SETSWAPCHAIN* pInArgs)
{
    auto* pMonitorContextWrapper = WdfObjectGet_MonitorContextWrapper(MonitorObject);
    pMonitorContextWrapper->pContext->AssignSwapChain(pInArgs->hSwapChain, pInArgs->RenderAdapterLuid, pInArgs->hNextSurfaceAvailable);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KastMonitorUnassignSwapChain(IDDCX_MONITOR MonitorObject)
{
    auto* pMonitorContextWrapper = WdfObjectGet_MonitorContextWrapper(MonitorObject);
    pMonitorContextWrapper->pContext->UnassignSwapChain();
    return STATUS_SUCCESS;
}

#pragma endregion
