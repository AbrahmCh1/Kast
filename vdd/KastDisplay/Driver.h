// Kast Virtual Display: controlador de pantallas virtuales (Indirect Display, IddCx) para Kast Link.
//
// Basado en el ejemplo IddSampleDriver de Microsoft (Windows-driver-samples, licencia MS-PL; ver ../LICENSE.txt).
#pragma once

#define NOMINMAX
// Los códigos NTSTATUS, de ntstatus.h (windows.h define algunos también).
#define WIN32_NO_STATUS
#include <windows.h>
#undef WIN32_NO_STATUS
#include <ntstatus.h>
#include <bugcodes.h>
#include <wudfwdm.h>
#include <wdf.h>
#include <iddcx.h>

#include <dxgi1_5.h>
#include <d3d11_2.h>
#include <avrt.h>
#include <wrl.h>

#include <map>
#include <memory>
#include <mutex>
#include <vector>

#include "../include/kastdisplay.h"

namespace Microsoft::WRL::Wrappers
{
    // Envoltorio para identificadores de hilo, como los demás de WRL.
    typedef HandleT<HandleTraits::HANDLENullTraits> Thread;
}

namespace Kast
{
    struct Mode
    {
        DWORD Width;
        DWORD Height;
        DWORD Hz;
    };

    // Dispositivo Direct3D en el adaptador que dibuja la pantalla.
    struct Direct3DDevice
    {
        explicit Direct3DDevice(LUID AdapterLuid);
        HRESULT Init();

        LUID AdapterLuid;
        Microsoft::WRL::ComPtr<IDXGIFactory5> DxgiFactory;
        Microsoft::WRL::ComPtr<IDXGIAdapter1> Adapter;
        Microsoft::WRL::ComPtr<ID3D11Device> Device;
        Microsoft::WRL::ComPtr<ID3D11DeviceContext> DeviceContext;
    };

    // Hilo que recoge los fotogramas de la pantalla (Windows los compone; Kast los captura como los de cualquier
    // monitor, así que aquí sólo se devuelven).
    class SwapChainProcessor
    {
    public:
        SwapChainProcessor(IDDCX_SWAPCHAIN hSwapChain, std::shared_ptr<Direct3DDevice> Device, HANDLE NewFrameEvent);
        ~SwapChainProcessor();

    private:
        static DWORD CALLBACK RunThread(LPVOID Argument);
        void Run();
        void RunCore();

        IDDCX_SWAPCHAIN m_hSwapChain;
        std::shared_ptr<Direct3DDevice> m_Device;
        HANDLE m_hAvailableBufferEvent;
        Microsoft::WRL::Wrappers::Thread m_hThread;
        Microsoft::WRL::Wrappers::Event m_hTerminateEvent;
    };

    class MonitorContext
    {
    public:
        MonitorContext(IDDCX_MONITOR Monitor, const Mode& Wanted);
        ~MonitorContext();

        void AssignSwapChain(IDDCX_SWAPCHAIN SwapChain, LUID RenderAdapter, HANDLE NewFrameEvent);
        void UnassignSwapChain();

        const Mode Wanted;

    private:
        IDDCX_MONITOR m_Monitor;
        std::unique_ptr<SwapChainProcessor> m_ProcessingThread;
    };

    // El adaptador: crea y quita pantallas cuando la aplicación las pide.
    class DeviceContext
    {
    public:
        explicit DeviceContext(WDFDEVICE WdfDevice);
        ~DeviceContext();

        void InitAdapter();
        void AdapterReady();

        NTSTATUS AddMonitor(const Mode& Wanted, ULONG* Id);
        NTSTATUS RemoveMonitor(ULONG Id);
        void RemoveAll();
        ULONG MonitorCount();
        // La aplicación sigue ahí (ver KAST_WATCHDOG_MS).
        void Touch();

    private:
        static DWORD CALLBACK WatchdogThread(LPVOID Argument);
        void Watchdog();

        struct Slot
        {
            IDDCX_MONITOR Monitor;
            ULONG Id;
            UINT Connector;
        };

        WDFDEVICE m_WdfDevice;
        IDDCX_ADAPTER m_Adapter = {};
        bool m_Ready = false;
        std::mutex m_Lock;
        std::vector<Slot> m_Slots;
        ULONG m_NextId = 1;
        ULONGLONG m_LastTouch = 0;
        Microsoft::WRL::Wrappers::Thread m_hWatchdog;
        Microsoft::WRL::Wrappers::Event m_hStop;
    };

    // Modos de cada pantalla por el número de serie de su EDID (al analizar el EDID no se sabe de qué pantalla es).
    void RegisterModes(ULONG Serial, const Mode& Wanted);
    void UnregisterModes(ULONG Serial);
    bool ModesForSerial(ULONG Serial, Mode* Wanted);
    // El modo pedido primero (el preferido) y los habituales más pequeños, para poder cambiarlo en Configuración.
    std::vector<Mode> ModeList(const Mode& Wanted);
}
