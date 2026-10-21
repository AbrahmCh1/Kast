import AVFoundation
import UIKit

/// Muestra el vídeo H.264 del otro equipo: decodificación y dibujo por hardware (AVSampleBufferDisplayLayer).
final class VideoView: UIView, VideoSink {
    override class var layerClass: AnyClass { AVSampleBufferDisplayLayer.self }
    private var display: AVSampleBufferDisplayLayer { layer as! AVSampleBufferDisplayLayer }
    /// Se usa desde el hilo del vídeo (no se toca la vista fuera del hilo principal).
    private var renderer: AVSampleBufferVideoRenderer!
    private var format: CMVideoFormatDescription?
    private var sps = Data(), pps = Data()

    override init(frame: CGRect) {
        super.init(frame: frame)
        display.videoGravity = .resize
        renderer = display.sampleBufferRenderer
        backgroundColor = .black
    }
    required init?(coder: NSCoder) { fatalError() }

    func enqueue(avcc: Data, sps newSps: Data?, pps newPps: Data?) {
        if let s = newSps, let p = newPps, s != sps || p != pps || format == nil {
            sps = s; pps = p
            format = Self.makeFormat(sps: s, pps: p)
        }
        guard let format, let sample = Self.makeSample(avcc, format: format) else { return }
        if renderer.status == .failed { renderer.flush() }
        renderer.enqueue(sample)
    }

    private static func makeFormat(sps: Data, pps: Data) -> CMVideoFormatDescription? {
        var desc: CMVideoFormatDescription?
        sps.withUnsafeBytes { s in
            pps.withUnsafeBytes { p in
                let ptrs = [s.bindMemory(to: UInt8.self).baseAddress!, p.bindMemory(to: UInt8.self).baseAddress!]
                let sizes = [sps.count, pps.count]
                _ = CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator: nil, parameterSetCount: 2, parameterSetPointers: ptrs,
                                                                        parameterSetSizes: sizes, nalUnitHeaderLength: 4, formatDescriptionOut: &desc)
            }
        }
        return desc
    }

    private static func makeSample(_ data: Data, format: CMVideoFormatDescription) -> CMSampleBuffer? {
        var block: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: data.count, blockAllocator: nil,
                                                 customBlockSource: nil, offsetToData: 0, dataLength: data.count, flags: 0,
                                                 blockBufferOut: &block) == noErr, let block else { return nil }
        let ok = data.withUnsafeBytes { CMBlockBufferReplaceDataBytes(with: $0.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: data.count) }
        guard ok == noErr else { return nil }
        var sample: CMSampleBuffer?
        var size = data.count
        guard CMSampleBufferCreateReady(allocator: nil, dataBuffer: block, formatDescription: format, sampleCount: 1, sampleTimingEntryCount: 0,
                                        sampleTimingArray: nil, sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample) == noErr,
              let sample else { return nil }
        // Sin marcas de tiempo: se muestra en cuanto llega (latencia mínima).
        if let att = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true), CFArrayGetCount(att) > 0 {
            let dict = unsafeBitCast(CFArrayGetValueAtIndex(att, 0), to: CFMutableDictionary.self)
            CFDictionarySetValue(dict, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                                 Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
        }
        return sample
    }
}
