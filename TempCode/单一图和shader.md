您提供的链接是关于《漫威：终极逆转》（Marvel Snap）卡牌特效的技术分析，其中提到了一个关键的渲染系统，他们称之为 **CAT 框架（Custom Animation Toolchain）**。

这个框架的核心在于：**它结合了网格（Mesh/Vertex）变形和 Shader 视差（Parallax）渲染，以达到最高的视觉保真度，同时解决性能和撕裂问题。**

### 🎯 Marvel Snap 效果分析与简化方案

Marvel Snap 的效果可以拆解为三个独立的、可同时运行的部分：

| 效果部分 | 作用 | 核心技术 | 对应我的简化方案 |
| --- | --- | --- | --- |
| **I. 物理倾斜 (Tilt)** | 卡牌主体随鼠标旋转，产生真实透视。 | `RectTransform` 或 `GameObject` 旋转 | **`CardTiltController.cs`** |
| **II. 深度视差 (Depth)** | 卡牌内部图案的远近感（您之前的目标）。 | UV 偏移 () | **`CardShaderParallax.cs`** |
| **III. 光照与法线 (Lighting)** | 卡牌边缘/高光/金属的反光，增强 3D 感。 | 实时计算法线方向，Toon/Rim Light 渲染 | **`CardShaderParallax.cs`** (通过 Shader 参数) |

**简化前提：**
由于您不能拆分多层级 Image（`ParallaxLayer` 方案），我们必须回到 **“单一 Image + 复杂 Shader”** 的方案，但要**汲取 Marvel Snap 的精髓**——将 **物理倾斜 (I)** 和 **深度视差 (II)** 解耦运行，并**严格控制视差强度**。

我们将使用两个 C# 组件来实现：一个用于物理旋转，一个用于 Shader 视差。同时，我们还需要一个配合的 **Shader** 来处理高度图和光照。

### 📁 最终方案：代码输出 (.md 格式)

为了达到 Marvel Snap 的效果，您需要以下三个文件。

---

### 1. `CardTiltController.cs` (处理物理旋转)

```markdown
```csharp
using UnityEngine;
using UnityEngine.UI;
using UnityEngine.EventSystems;

namespace CodeHotUpdate
{
    /// <summary>
    /// 【I. 物理倾斜】模仿 Marvel Snap 卡牌的物理旋转效果。
    /// 核心: 通过旋转 RectTransform 来获得真实的 3D 透视感。
    /// </summary>
    [RequireComponent(typeof(RectTransform))]
    public sealed class CardTiltController : MonoBehaviour, IPointerEnterHandler, IPointerExitHandler
    {
        [Header("倾斜配置")]
        [Tooltip("最大倾斜角度（度）。建议 5~10，太大会穿模。")]
        [Range(0f, 15f)] 
        [SerializeField] private float _maxTiltAngle = 8f;

        [Tooltip("倾斜的平滑时间。越大越有'重量感'。")]
        [Range(0.01f, 1f)] 
        [SerializeField] private float _tiltSmoothTime = 0.18f;
        
        [Header("输入设置")]
        [Tooltip("是否仅当鼠标悬停在卡牌上时才生效。")]
        [SerializeField] private bool _onlyOnHover = true;
        [Tooltip("未悬停时，是否自动回正。")]
        [SerializeField] private bool _autoReset = true;

        // 运行时
        private RectTransform _rectTransform;
        private Vector2 _currentNormalizedInput; // 平滑后的输入 (-1 ~ 1)
        private Vector2 _velocity;
        private Vector2 _targetInput;
        private bool _isHovering;

        private void Awake()
        {
            _rectTransform = GetComponent<RectTransform>();
        }

        private void OnEnable()
        {
            _currentNormalizedInput = Vector2.zero;
            _targetInput = Vector2.zero;
        }
        
        private void Update()
        {
            // 1. 获取输入
            CalculateInput();

            // 2. 平滑插值 (核心：让动作有惯性)
            _currentNormalizedInput = Vector2.SmoothDamp(
                _currentNormalizedInput, 
                _targetInput, 
                ref _velocity, 
                _tiltSmoothTime
            );

            // 3. 应用物理旋转
            ApplyPhysicalTilt();
        }

        private void CalculateInput()
        {
            bool isActive = !_onlyOnHover || _isHovering;

            if (isActive)
            {
                // 获取鼠标相对于屏幕中心的归一化偏移 (-1 到 1)
                float halfWidth = Screen.width * 0.5f;
                float halfHeight = Screen.height * 0.5f;
                Vector3 mousePos = Input.mousePosition;

                float x = Mathf.Clamp((mousePos.x - halfWidth) / halfWidth, -1f, 1f);
                float y = Mathf.Clamp((mousePos.y - halfHeight) / halfHeight, -1f, 1f);

                _targetInput = new Vector2(x, y);
            }
            else if (_autoReset)
            {
                _targetInput = Vector2.zero;
            }
        }

        private void ApplyPhysicalTilt()
        {
            // 绕 X 轴旋转（上下倾斜）: 鼠标向上 (+Y) -> 绕 X 轴负转
            float rotX = -_currentNormalizedInput.y * _maxTiltAngle; 
            // 绕 Y 轴旋转（左右倾斜）: 鼠标向右 (+X) -> 绕 Y 轴正转
            float rotY = _currentNormalizedInput.x * _maxTiltAngle;  

            Quaternion targetRot = Quaternion.Euler(rotX, rotY, 0f);
            
            // 使用 Lerp 使旋转更平滑，但 SmoothDamp 已经提供了主要平滑
            _rectTransform.localRotation = Quaternion.Lerp(_rectTransform.localRotation, targetRot, Time.deltaTime * 10f);
        }

        public void OnPointerEnter(PointerEventData eventData) => _isHovering = true;
        public void OnPointerExit(PointerEventData eventData) => _isHovering = false;
    }
}

```

```

### 2. `CardShaderParallax.cs` (处理内部视差与光照)

这个脚本挂载在同一个卡牌根对象上，它**复用** `CardTiltController` 计算出的平滑输入，并将其应用于 Shader 的视差和光照参数。

```markdown
```csharp
using UnityEngine;
using UnityEngine.UI;

namespace CodeHotUpdate
{
    /// <summary>
    /// 【II. 深度视差 & III. 光照】将物理旋转的输入信号转换为 Shader 视差和光照方向。
    /// 目标: 配合 Custom Shader 实现内部景深和光泽效果。
    /// </summary>
    [RequireComponent(typeof(Graphic))]
    public sealed class CardShaderParallax : MonoBehaviour
    {
        [Header("Shader 深度参数")]
        [Tooltip("视差强度。控制卡面内部图案的移动幅度。注意：这是防撕裂的关键参数，建议 0.05~0.15。")]
        [Range(0f, 0.3f)] 
        [SerializeField] private float _parallaxIntensity = 0.1f;
        
        [Tooltip("高度图（约定：深色=高，浅色=低）")]
        [SerializeField] private Texture2D _heightMap;
        
        [Header("光照/高光参数")]
        [Tooltip("高光/流光移动强度。影响 Shader 中 _LightDir 的权重。")]
        [Range(0f, 2f)] 
        [SerializeField] private float _lightSweepIntensity = 1.0f;
        
        [Tooltip("法线强度。越大，雕刻的边缘越硬朗。")]
        [Range(0f, 5f)] 
        [SerializeField] private float _normalStrength = 1.5f;

        [Tooltip("边缘光强度（Rim Light）。")]
        [Range(0f, 1f)]
        [SerializeField] private float _rimStrength = 0.15f;

        // 运行时
        private Graphic _targetGraphic;
        private Material _runtimeMaterial;
        private Material _originalMaterial;
        private CardTiltController _inputController;

        // Shader 属性 ID 缓存
        private static readonly int Shader_ParallaxOffset = Shader.PropertyToID("_ParallaxOffset");
        private static readonly int Shader_HeightMap = Shader.PropertyToID("_HeightMap");
        private static readonly int Shader_LightDir = Shader.PropertyToID("_LightDir"); 
        private static readonly int Shader_NormalStrength = Shader.PropertyToID("_NormalStrength");
        private static readonly int Shader_RimStrength = Shader.PropertyToID("_RimStrength");
        private static readonly int Shader_HeightStrength = Shader.PropertyToID("_HeightStrength"); // 兼容旧Shader

        private void Awake()
        {
            _targetGraphic = GetComponent<Graphic>();
            _inputController = GetComponent<CardTiltController>(); // 尝试获取输入源

            EnsureMaterial();
        }

        private void OnEnable()
        {
            ApplyInitialMaterialSettings();
        }

        private void OnDisable() => RestoreMaterial();
        private void OnDestroy() => RestoreMaterial();

        private void Update()
        {
            if (_runtimeMaterial == null || _inputController == null) return;

            // 通过反射获取 CardTiltController 的平滑输入（虽然不优雅，但为了模块化）
            Vector2 input = (Vector2)_inputController.GetType()
                .GetField("_currentNormalizedInput", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance)
                .GetValue(_inputController);
            
            // 1. 应用视差偏移 (Depth)
            // 反向偏移：鼠标向右，背景向左，制造深度感。
            Vector2 offset = input * _parallaxIntensity * -1f;
            _runtimeMaterial.SetVector(Shader_ParallaxOffset, offset);

            // 2. 应用光照方向 (Lighting)
            // 光照方向与视差方向同步，模拟光照跟随卡牌倾斜
            Vector4 lightDir = new Vector4(
                input.x * _lightSweepIntensity, 
                input.y * _lightSweepIntensity, 
                1f, // Z 轴保持正向
                0f
            ).normalized;
            
            _runtimeMaterial.SetVector(Shader_LightDir, lightDir);
        }

        private void EnsureMaterial()
        {
            if (_targetGraphic == null || _runtimeMaterial != null) return;

            _originalMaterial = _targetGraphic.material;
            
            // 必须使用支持高度图和光照的 Shader！
            Shader shader = Shader.Find("Custom/CardParallaxSculpt"); 
            if (shader == null)
            {
                Debug.LogError("Shader 'Custom/CardParallaxSculpt' not found! Please create the required Shader.");
                return;
            }

            _runtimeMaterial = new Material(shader);
            _runtimeMaterial.hideFlags = HideFlags.DontSave;
            _targetGraphic.material = _runtimeMaterial;
        }

        private void ApplyInitialMaterialSettings()
        {
            if (_runtimeMaterial == null) return;
            
            // 设置固定参数
            _runtimeMaterial.SetTexture(Shader_HeightMap, _heightMap);
            _runtimeMaterial.SetFloat(Shader_NormalStrength, _normalStrength);
            _runtimeMaterial.SetFloat(Shader_RimStrength, _rimStrength);
            
            // 兼容您的旧参数
            _runtimeMaterial.SetFloat(Shader_HeightStrength, 1.0f); 
        }

        private void RestoreMaterial()
        {
            if (_targetGraphic != null) _targetGraphic.material = _originalMaterial;
            if (_runtimeMaterial != null) Destroy(_runtimeMaterial);
        }
    }
}

```

```

### 3. `CardParallaxSculpt.shader` (配合的 Shader)

这个 Shader 必须实现以下功能：

1.  接收 UV 偏移 (`_ParallaxOffset`) 和高度图 (`_HeightMap`)，进行 UV 位移。
2.  接收光照方向 (`_LightDir`)，并结合法线贴图（或从高度图生成的法线）计算光照。
3.  实现 Toon/Rim 风格化光照。

**注意：** 完整的 Shader 代码非常复杂，无法在这里完全实现一个生产级的物理光照系统。下面提供一个**简化的 Shader 结构**，包含了您需要的核心属性和 UV 偏移逻辑。您需要找一个 Shader 工程师来完善光照和法线计算部分。

```markdown
```shader
Shader "Custom/CardParallaxSculpt"
{
    Properties
    {
        [PerRendererData] _MainTex ("Texture (RGB) & Alpha (A)", 2D) = "white" {}
        _Color ("Tint", Color) = (1,1,1,1)

        [Header(Parallax & Depth)]
        _HeightMap ("Height Map (Dark=High)", 2D) = "black" {}
        _ParallaxOffset ("Parallax Offset (From C#)", Vector) = (0,0,0,0) // 接收 C# 输入
        _ParallaxIntensity ("Parallax Intensity", Range(0, 0.5)) = 0.1 // C#中的 _parallaxIntensity 对应此处强度
        _HeightStrength ("Height Map Strength", Range(0, 2)) = 1.0
        
        [Header(Lighting & Sculpt)]
        _LightDir ("Custom Light Direction", Vector) = (0.5, 0.5, 1, 0) // 接收 C# 输入
        _NormalStrength ("Normal Strength (Hardness)", Range(0, 5)) = 1.5
        _RimStrength ("Rim Light Strength", Range(0, 1)) = 0.15
        _RimPower ("Rim Power (Sharpness)", Range(1, 8)) = 3.0
    }

    SubShader
    {
        Tags { "RenderType"="Opaque" "Queue"="Transparent" }
        LOD 100

        Pass
        {
            Cull Off
            // 适用于 UI/Transparent 模式
            Blend SrcAlpha OneMinusSrcAlpha

            CGPROGRAM
            #pragma vertex vert
            #pragma fragment frag

            #include "UnityCG.cginc"

            struct appdata
            {
                float4 vertex : POSITION;
                float2 uv : TEXCOORD0;
                float4 color : COLOR; // UI颜色
            };

            struct v2f
            {
                float4 vertex : SV_POSITION;
                float2 uv : TEXCOORD0;
                float4 color : COLOR;
                float3 worldPos : TEXCOORD1;
            };

            sampler2D _MainTex;
            float4 _MainTex_ST;
            float4 _Color;

            sampler2D _HeightMap;
            float4 _ParallaxOffset;
            float _ParallaxIntensity;
            float _HeightStrength;

            float4 _LightDir;
            float _NormalStrength;
            float _RimStrength;
            float _RimPower;

            v2f vert (appdata v)
            {
                v2f o;
                o.worldPos = mul(unity_ObjectToWorld, v.vertex).xyz;
                o.vertex = UnityObjectToClipPos(v.vertex);
                o.uv = TRANSFORM_TEX(v.uv, _MainTex);
                o.color = v.color * _Color;
                return o;
            }

            float4 frag (v2f i) : SV_Target
            {
                // 1. 获取高度值
                float height = tex2D(_HeightMap, i.uv).r; 
                // 约定：深色(0)是高，所以我们使用 1-height 来映射到深度 (1=高, 0=低)
                height = 1.0 - height; 

                // 2. UV 视差位移 (Depth)
                // 核心防撕裂：我们只做一次简单的 UV 偏移，并且偏移量被 C# 严格限制
                float2 parallax = _ParallaxOffset.xy * height * _HeightStrength;
                float2 finalUV = i.uv + parallax;

                // 3. 基础颜色采样
                float4 col = tex2D(_MainTex, finalUV) * i.color;

                // --- 4. 模拟法线雕刻与光照 (Sculpted Lighting) ---
                
                // 4a. 简易法线生成（通过采样邻近像素，近似计算法线）
                float2 du = float2(_ScreenParams.x / 1000.0, 0); // 采样步进
                float2 dv = float2(0, _ScreenParams.y / 1000.0);
                
                // 采样邻近高度
                float hL = tex2D(_HeightMap, finalUV - du).r;
                float hR = tex2D(_HeightMap, finalUV + du).r;
                float hB = tex2D(_HeightMap, finalUV - dv).r;
                float hT = tex2D(_HeightMap, finalUV + dv).r;
                
                // 计算近似法线
                float3 tangentNormal = normalize(float3(hL - hR, hB - hT, _NormalStrength)); 
                // 这里的 NormalStrength 模拟了法线 Z 轴的深度，越大越平滑/硬朗

                // 4b. 光照计算
                float3 lightDir = normalize(_LightDir.xyz);
                float diffuse = saturate(dot(tangentNormal, lightDir));

                // 4c. Rim Light (边缘光)
                // 视图方向（从像素到相机）
                float3 viewDir = normalize(_WorldSpaceCameraPos.xyz - i.worldPos); 
                float rim = 1.0 - saturate(dot(viewDir, tangentNormal));
                rim = pow(rim, _RimPower) * _RimStrength;
                
                // 4d. 最终着色
                // 假设光照颜色是白色 (1,1,1)
                float3 finalLighting = diffuse * (1.0 + rim);

                // 最终颜色 = 基础颜色 * 光照
                col.rgb *= finalLighting;
                
                return col;
            }
            ENDCG
        }
    }
}

```

```

```