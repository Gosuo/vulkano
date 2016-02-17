use std::io::Error as IoError;
use std::io::Read;

pub use parse::ParseError;

mod enums;
mod parse;

pub fn reflect<R>(name: &str, mut spirv: R) -> Result<String, Error>
    where R: Read
{
    let mut data = Vec::new();
    try!(spirv.read_to_end(&mut data));

    // now parsing the document
    let doc = try!(parse::parse_spirv(&data));

    let mut output = String::new();

    {
        // contains the data that was passed as input to this function
        let spirv_data = data.iter().map(|&byte| byte.to_string())
                             .collect::<Vec<String>>()
                             .join(", ");

        // writing the header
        output.push_str(&format!(r#"
pub struct {name} {{
    shader: ::std::sync::Arc<::vulkano::shader::ShaderModule>,
}}

impl {name} {{
    /// Loads the shader in Vulkan as a `ShaderModule`.
    #[inline]
    pub fn load(device: &::std::sync::Arc<::vulkano::device::Device>) -> {name} {{

        "#, name = name));

        // checking whether each required capability is supported by the vulkan implementation
        for i in doc.instructions.iter() {
            if let &parse::Instruction::Capability(ref cap) = i {
                if let Some(cap) = capability_name(cap) {
                    output.push_str(&format!(r#"
                        if !device.enabled_features().{cap} {{
                            panic!("capability not supported")  // FIXME: error
                            //return Err(CapabilityNotSupported);
                        }}"#, cap = cap));
                }
            }
        }

        // follow-up of the header
        output.push_str(&format!(r#"
        unsafe {{
            let data = [{spirv_data}];

            {name} {{
                shader: ::vulkano::shader::ShaderModule::new(device, &data).unwrap()    // FIXME: try!()
            }}
        }}
    }}

    /// Returns the module that was created.
    #[allow(dead_code)]
    #[inline]
    pub fn module(&self) -> &::std::sync::Arc<::vulkano::shader::ShaderModule> {{
        &self.shader
    }}
        "#, name = name, spirv_data = spirv_data));

        // writing one method for each entry point of this module
        for instruction in doc.instructions.iter() {
            if let &parse::Instruction::EntryPoint { .. } = instruction {
                output.push_str(&write_entry_point(&doc, instruction));
            }
        }

        // footer
        output.push_str(&format!(r#"
}}
        "#));
    }

    // TODO: remove
    println!("{:#?}", doc);

    Ok(output)
}

#[derive(Debug)]
pub enum Error {
    IoError(IoError),
    ParseError(ParseError),
}

impl From<IoError> for Error {
    #[inline]
    fn from(err: IoError) -> Error {
        Error::IoError(err)
    }
}

impl From<ParseError> for Error {
    #[inline]
    fn from(err: ParseError) -> Error {
        Error::ParseError(err)
    }
}

fn write_entry_point(doc: &parse::Spirv, instruction: &parse::Instruction) -> String {
    let (execution, ep_name, interface) = match instruction {
        &parse::Instruction::EntryPoint { ref execution, id, ref name, ref interface } => {
            (execution, name, interface)
        },
        _ => unreachable!()
    };

    let (ty, f_name) = match *execution {
        enums::ExecutionModel::ExecutionModelVertex => {
            let mut input_types = Vec::new();

            // TODO: sort types by location

            for interface in interface.iter() {
                for i in doc.instructions.iter() {
                    match i {
                        &parse::Instruction::Variable { result_type_id, result_id,
                                    storage_class: enums::StorageClass::StorageClassInput, .. }
                                    if &result_id == interface =>
                        {
                            input_types.push(type_from_id(doc, result_type_id));
                        },
                        _ => ()
                    }
                }
            }

            let t = format!("::vulkano::shader::VertexShaderEntryPoint<({input})>",
                            input = input_types.join(", ") + ",");
            (t, "vertex_shader_entry_point")
        },

        enums::ExecutionModel::ExecutionModelTessellationControl => {
            (format!("::vulkano::shader::TessControlShaderEntryPoint"), "")
        },

        enums::ExecutionModel::ExecutionModelTessellationEvaluation => {
            (format!("::vulkano::shader::TessEvaluationShaderEntryPoint"), "")
        },

        enums::ExecutionModel::ExecutionModelGeometry => {
            (format!("::vulkano::shader::GeometryShaderEntryPoint"), "")
        },

        enums::ExecutionModel::ExecutionModelFragment => {
            let mut output_types = Vec::new();

            for interface in interface.iter() {
                for i in doc.instructions.iter() {
                    match i {
                        &parse::Instruction::Variable { result_type_id, result_id,
                                    storage_class: enums::StorageClass::StorageClassOutput, .. }
                                    if &result_id == interface =>
                        {
                            output_types.push(type_from_id(doc, result_type_id));
                        },
                        _ => ()
                    }
                }
            }

            let t = format!("::vulkano::shader::FragmentShaderEntryPoint<({output})>",
                            output = output_types.join(", ") + ",");
            (t, "fragment_shader_entry_point")
        },

        enums::ExecutionModel::ExecutionModelGLCompute => {
            (format!("::vulkano::shader::ComputeShaderEntryPoint"), "compute_shader_entry_point")
        },

        enums::ExecutionModel::ExecutionModelKernel => panic!("Kernels are not supported"),
    };

    format!(r#"
    /// Returns a logical struct describing the entry point named `{ep_name}`.
    #[inline]
    pub fn {ep_name}_entry_point(&self) -> {ty} {{
        unsafe {{
            #[allow(dead_code)]
            static NAME: [u8; {ep_name_lenp1}] = [{encoded_ep_name}, 0];     // "{ep_name}"
            self.shader.{f_name}(::std::ffi::CStr::from_ptr(NAME.as_ptr() as *const _))
        }}
    }}
            "#, ep_name = ep_name, ep_name_lenp1 = ep_name.chars().count() + 1, ty = ty,
                encoded_ep_name = ep_name.chars().map(|c| (c as u32).to_string())
                                         .collect::<Vec<String>>().join(", "),
                f_name = f_name)
}

fn type_from_id(doc: &parse::Spirv, searched: u32) -> String {
    for instruction in doc.instructions.iter() {
        match instruction {
            &parse::Instruction::TypeVoid { result_id } if result_id == searched => {
                return "()".to_owned()
            },
            &parse::Instruction::TypeBool { result_id } if result_id == searched => {
                return "bool".to_owned()
            },
            &parse::Instruction::TypeInt { result_id, width, signedness } if result_id == searched => {
                return "i32".to_owned()
            },
            &parse::Instruction::TypeFloat { result_id, width } if result_id == searched => {
                return "f32".to_owned()
            },
            &parse::Instruction::TypeVector { result_id, component_id, count } if result_id == searched => {
                let t = type_from_id(doc, component_id);
                return format!("[{}; {}]", t, count);
            },
            &parse::Instruction::TypeImage { result_id, sampled_type_id, ref dim, depth, arrayed, ms, sampled, ref format, ref access } if result_id == searched => {
                return format!("{}{}Texture{:?}{}{:?}",
                    if ms { "Multisample" } else { "" },
                    if depth == Some(true) { "Depth" } else { "" },
                    dim,
                    if arrayed { "Array" } else { "" },
                    format);
            },
            &parse::Instruction::TypeSampledImage { result_id, image_type_id } if result_id == searched => {
                return type_from_id(doc, image_type_id);
            },
            &parse::Instruction::TypeArray { result_id, type_id, length_id } if result_id == searched => {
                let t = type_from_id(doc, type_id);
                let len = doc.instructions.iter().filter_map(|e| {
                    match e { &parse::Instruction::Constant { result_id, ref data, .. } if result_id == length_id => Some(data.clone()), _ => None }
                }).next().expect("failed to find array length");
                let len = len.iter().rev().fold(0u64, |a, &b| (a << 32) | b as u64);
                return format!("[{}; {}]", t, len);       // FIXME:
            },
            &parse::Instruction::TypeRuntimeArray { result_id, type_id } if result_id == searched => {
                let t = type_from_id(doc, type_id);
                return format!("[{}]", t);
            },
            &parse::Instruction::TypeStruct { result_id, ref member_types } if result_id == searched => {
                let name = name_from_id(doc, result_id);
                let members = member_types.iter().enumerate().map(|(offset, &member)| {
                    let ty = type_from_id(doc, member);
                    let name = member_name_from_id(doc, result_id, offset as u32);
                    format!("\t{}: {}", name, ty)
                }).collect::<Vec<_>>();
                return format!("struct {} {{\n{}\n}}", name, members.join(",\n"));
            },
            &parse::Instruction::TypeOpaque { result_id, ref name } if result_id == searched => {
                return "<opaque>".to_owned();
            },
            &parse::Instruction::TypePointer { result_id, type_id, .. } if result_id == searched => {
                return type_from_id(doc, type_id);
            },
            _ => ()
        }
    }

    panic!("Type #{} not found", searched)
}

fn name_from_id(doc: &parse::Spirv, searched: u32) -> String {
    doc.instructions.iter().filter_map(|i| {
        if let &parse::Instruction::Name { target_id, ref name } = i {
            if target_id == searched {
                Some(name.clone())
            } else {
                None
            }
        } else {
            None
        }
    }).next().and_then(|n| if !n.is_empty() { Some(n) } else { None })
      .unwrap_or("__unnamed".to_owned())
}

fn member_name_from_id(doc: &parse::Spirv, searched: u32, searched_member: u32) -> String {
    doc.instructions.iter().filter_map(|i| {
        if let &parse::Instruction::MemberName { target_id, member, ref name } = i {
            if target_id == searched && member == searched_member {
                Some(name.clone())
            } else {
                None
            }
        } else {
            None
        }
    }).next().and_then(|n| if !n.is_empty() { Some(n) } else { None })
      .unwrap_or("__unnamed".to_owned())
}

/// Returns the name of the Vulkan something that corresponds to an `OpCapability`.
///
/// Returns `None` if irrelevant.
// TODO: this function is a draft, as the actual names may not be the same
fn capability_name(cap: &enums::Capability) -> Option<&'static str> {
    match *cap {
        enums::Capability::CapabilityMatrix => None,        // always supported
        enums::Capability::CapabilityShader => None,        // always supported
        enums::Capability::CapabilityGeometry => Some("geometryShader"),
        enums::Capability::CapabilityTessellation => Some("tessellationShader"),
        enums::Capability::CapabilityAddresses => panic!(), // not supported
        enums::Capability::CapabilityLinkage => panic!(),   // not supported
        enums::Capability::CapabilityKernel => panic!(),    // not supported
        enums::Capability::CapabilityVector16 => panic!(),  // not supported
        enums::Capability::CapabilityFloat16Buffer => panic!(), // not supported
        enums::Capability::CapabilityFloat16 => panic!(),   // not supported
        enums::Capability::CapabilityFloat64 => Some("shaderFloat64"),
        enums::Capability::CapabilityInt64 => Some("shaderInt64"),
        enums::Capability::CapabilityInt64Atomics => panic!(),  // not supported
        enums::Capability::CapabilityImageBasic => panic!(),    // not supported
        enums::Capability::CapabilityImageReadWrite => panic!(),    // not supported
        enums::Capability::CapabilityImageMipmap => panic!(),   // not supported
        enums::Capability::CapabilityPipes => panic!(), // not supported
        enums::Capability::CapabilityGroups => panic!(),    // not supported
        enums::Capability::CapabilityDeviceEnqueue => panic!(), // not supported
        enums::Capability::CapabilityLiteralSampler => panic!(),    // not supported
        enums::Capability::CapabilityAtomicStorage => panic!(), // not supported
        enums::Capability::CapabilityInt16 => Some("shaderInt16"),
        enums::Capability::CapabilityTessellationPointSize => Some("shaderTessellationAndGeometryPointSize"),
        enums::Capability::CapabilityGeometryPointSize => Some("shaderTessellationAndGeometryPointSize"),
        enums::Capability::CapabilityImageGatherExtended => Some("shaderImageGatherExtended"),
        enums::Capability::CapabilityStorageImageMultisample => Some("shaderStorageImageMultisample"),
        enums::Capability::CapabilityUniformBufferArrayDynamicIndexing => Some("shaderUniformBufferArrayDynamicIndexing"),
        enums::Capability::CapabilitySampledImageArrayDynamicIndexing => Some("shaderSampledImageArrayDynamicIndexing"),
        enums::Capability::CapabilityStorageBufferArrayDynamicIndexing => Some("shaderStorageBufferArrayDynamicIndexing"),
        enums::Capability::CapabilityStorageImageArrayDynamicIndexing => Some("shaderStorageImageArrayDynamicIndexing"),
        enums::Capability::CapabilityClipDistance => Some("shaderClipDistance"),
        enums::Capability::CapabilityCullDistance => Some("shaderCullDistance"),
        enums::Capability::CapabilityImageCubeArray => Some("imageCubeArray"),
        enums::Capability::CapabilitySampleRateShading => Some("sampleRateShading"),
        enums::Capability::CapabilityImageRect => panic!(), // not supported
        enums::Capability::CapabilitySampledRect => panic!(),   // not supported
        enums::Capability::CapabilityGenericPointer => panic!(),    // not supported
        enums::Capability::CapabilityInt8 => panic!(),  // not supported
        enums::Capability::CapabilityInputAttachment => None,       // always supported
        enums::Capability::CapabilitySparseResidency => Some("shaderResourceResidency"),
        enums::Capability::CapabilityMinLod => Some("shaderResourceMinLod"),
        enums::Capability::CapabilitySampled1D => None,        // always supported
        enums::Capability::CapabilityImage1D => None,        // always supported
        enums::Capability::CapabilitySampledCubeArray => Some("imageCubeArray"),
        enums::Capability::CapabilitySampledBuffer => None,         // always supported
        enums::Capability::CapabilityImageBuffer => None,        // always supported
        enums::Capability::CapabilityImageMSArray => Some("shaderStorageImageMultisample"),
        enums::Capability::CapabilityStorageImageExtendedFormats => Some("shaderStorageImageExtendedFormats"),
        enums::Capability::CapabilityImageQuery => None,        // always supported
        enums::Capability::CapabilityDerivativeControl => None,        // always supported
        enums::Capability::CapabilityInterpolationFunction => Some("sampleRateShading"),
        enums::Capability::CapabilityTransformFeedback => panic!(), // not supported
        enums::Capability::CapabilityGeometryStreams => panic!(),   // not supported
        enums::Capability::CapabilityStorageImageReadWithoutFormat => Some("shaderStorageImageReadWithoutFormat"),
        enums::Capability::CapabilityStorageImageWriteWithoutFormat => Some("shaderStorageImageWriteWithoutFormat"),
        enums::Capability::CapabilityMultiViewport => Some("multiViewport"),
    }
}