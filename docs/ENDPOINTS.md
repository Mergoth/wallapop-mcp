> Historical upstream endpoint research. Python paths and browser examples below refer to the pre-Rust implementation retained in Git history. Current code and contracts: [architecture](ARCHITECTURE.md), [MCP](MCP.md), [Rust API](API.md).

# Endpoints

Catálogo de todos los endpoints que usa la CLI, cómo se extrajeron y a qué
llamada del código corresponden. **Ninguna llamada necesita navegador**:
todas son HTTP plano vía `wallapoplib/http.py` (`http_request`), con el token
de sesión como `Authorization: Bearer <access_token>`.

## Convenciones

- Base de la API: `https://api.wallapop.com` (`API_BASE = /api/v3`).
- Los endpoints `BFF` (Next.js de la web) no llevan `/api/v3`: viven bajo
  `/bff/...` directamente en la raíz del host.
- `Auth` = requiere `Authorization: Bearer`. La CLI añade además
  `X-DeviceID` si hay cookie `device_id`, y marca la app con
  `Origin: https://es.wallapop.com`, `X-Product: wallapop`,
  `X-AppVersion: 826560`, `X-DeviceOS: 0`.
- Métodos del cliente (en `wallapoplib/client.py`): `_request()` (anónimo) y
  `_authed()` (con auth + refresh automático de la sesión web si `401`);
  `_bff()` hace lo mismo pero bajo la raíz sin `/api/v3`.

### Identificadores de app (constantes de `config.py`)

Definidas en `wallapoplib/config.py`. **No son secretos**: la web las expone en
su bundle JS y las usa cualquier navegador, por eso se pueden versionar.

- `API_BASE` (`https://api.wallapop.com`) y `API_ROOT` (raíz sin `/api/v3`,
  para los endpoints BFF de Next.js).
- `WEB_BASE` (`https://es.wallapop.com`): origen web; va en `Origin`/`Referer`.
- `PUB_KEY` (`pub-c-...`) y `SUB_KEY` (`sub-c-...`): llaves de *publicar* y
  *suscribir* de la app de PubNub de Wallapop (su chat en tiempo real).
  Identifican el espacio de canales, no autentican. Que el publish REST con la
  `pub key` sea abierto es lo que permite mandar el mensaje de chat sin
  navegador.
- `PUB_ORIGIN` (`ps7.pndsn.com`): el broker de PubNub al que se publica.
- `PNSDK` (`PubNub-JS-Web/10.2.6`): cadena de auto-identificación del SDK en
  cada petición.
- `WEB_APP_VERSION` (`8.2656.0`): la build de la web que se simula; el API la
  usa para identificar al cliente (`X-AppVersion: 826560`).

Lo que sí es sensible (tokens y cookies) no vive en el repo: está en
`~/.config/wallapop_cli/{credentials,session}.json`, fuera de git.

**Extracción de endpoints (historias de origen):**

- Los endpoints de búsqueda actuales (`/search/components` + `/search/section`)
  se descubrieron inspeccionando las peticiones XHR de la web en el navegador y
  cruzando con el bundle JS. `/general/search` es el flujo antiguo que la CLI
  mantiene como fallback funcional.
- Los endpoints de **ofertas** se recuperaron del bundle de la app web:
  `https://web-static.wallapop.com/angular/es/chunk-*.js` (los hashes rotan
  con la versión de la app `8.2656.0`). En concretos:
  - `chunk-WF7BPLHU.js` — el servicio de ofertas define las rutas:
    `R="bff/delivery/offer-details"`, `h="api/v3/delivery/buyer/offers"`,
    `P="api/v3/delivery/offers"`, `U = .../{id}/status`,
    `v = .../{id}/offer-price`.
  - `chunk-THOUZVB2.js` — el componente del modal OTS define
    `submitOfferResponse` (mapea a `ACCEPTED`/`DECLINED` sobre `{id}/status`)
    y `submitCounterOffer` (sobre `{id}/offer-price`).
  Se validaron en vivo: un `PATCH status` sobre una oferta ya cerrada devuelve
  `409 invalid offer status` (es decir, el endpoint existe y valida de verdad).
- Los endpoints `bff/messaging/*` son de la SPA Next.js; el inbox devuelve los
  mensajes más recientes **primero** (hay que ordenar por `timestamp`).
- El envío de chat por REST PubNub se extrajo observando a la web: el mensaje
  se publica a `ps7.pndsn.com` con `auth` = token de
  `/instant-messaging/token`, `pnsdk=PubNub-JS-Web/10.2.6`. Acknowledgment
  `[1,"Sent",<timetoken>]`.

---

## Autenticación

### POST `/api/v3/access/login`

Login email+password de la app.

- Body: `{"email": ..., "password": ..., "client_id": "wallapop"}`
- No requiere token previo.
- Respuesta: `data.access_token`, `data.refresh_token`, `data.user`.
- Código: `Wallapop.login()`.

### POST `/api/v3/access/refresh`

Renovación de token.

- Body: `{"refresh_token": ..., "client_id": "wallapop"}`
- Respuesta: `data.access_token` (+ nuevo `refresh_token`).
- Código: `Wallapop.try_refresh()`.

### GET `https://es.wallapop.com/api/auth/session` [cookie]

Convierte la sesión del navegador (NextAuth) en un JWT usable.

- Necesita la cookie `__Secure-next-auth.session-token` (junto a
  `device_id`, `device_fgp`, `accessToken` si las hay).
- Respuesta: JSON con el campo `token` (JWT de la API). Es `accessToken`.
- Código: `refresh_web_session()`.

---

## Búsqueda

### GET `/api/v3/search/components`

Bootstrap de una búsqueda.

- Params: `keywords`, `source=search_box`, `order_by`
  (`most_relevance` | `newest` | `price_low_to_high` | `price_high_to_low` |
  `closest`), `filters_source=search_box`, opcional `category_id`,
  `subcategory_ids`, `condition`, `min_sale_price`, `max_sale_price`,
  `is_shippable=true`, `latitude`, `longitude`.
- **`category_id` debe ser top-level** (100, 12800, 24200...). Un id de
  node intermedio (p. ej. `24203`, hijo de `24200`) se acepta pero devuelve
  **0 resultados** en silencio. La CLI lo normaliza a su raíz con
  `Wallapop().top_level_category()`.
- **`subcategory_ids` sin `category_id` devuelve 400** en este endpoint.
  La CLI resuelve la raíz automáticamente con
  `Wallapop().category_for_subcategory()` (p. ej. `10088` → `24200`).
- De su respuesta se extrae `search_id` (clave para paginar).
- Código: `Wallapop.search()`.

### GET `/api/v3/search/section`

Página de resultados (lo que la web monta tras el bootstrap).

- Params: los del paso anterior + `search_id`, `section_type=organic_search_results`,
  `source=deep_link`; para página 2+ añadir `next_page` (cursor que devuelve
  la propia API en el campo `next_page`).
- Respuesta: `...items[]` con los anuncios sin normalizar.
- **Página 1 puede venir vacía pero con `next_page`** — ocurre con
  `is_shippable=true` + `price_low_to_high`: la API deja la primera página
  como placeholder (los items con envío real quedan a partir de la página 2).
  `_search_sections()` sigue el cursor aunque la página venga sin items y no
  descuenta esas páginas vacías del presupuesto de `--pages`.
- Código: `Wallapop._search_sections()`.

### GET `/api/v3/categories?context=search`

Catálogo completo de categorías para el buscador — **la fuente canónica del
menú lateral de la web** (`es.wallapop.com/search`). Verificado con Playwright
contra el sidebar real (véase "Verificación" abajo).

- Params: `context=search` (obligatorio).
- Respuesta: `{"categories": [...]}` con nodos `{id, name, vertical_id,
  attributes, subcategories[]}`. `subcategories` es **recursivo**, con
  `parent_id` en cada nivel; los nodos hoja tienen `subcategories: []`.
- Los nombres se devuelven **localizados** según el locale de la petición
  (Acepta-Language): sin locale el API responde en español (`Coches`, `Motos`…),
  mientras el navegador en `en-*` muestra `Cars`, `Motorbike`… Mismo `id`.
- Código: `Wallapop.categories()`.
- Snapshot del árbol completo validado: `docs/categories.json`.

Las 18 categorías **top-level** (id | nombre, localizadas a ES):

| id | nombre |
|----|--------|
| 100 | Coches |
| 14000 | Motos |
| 12800 | Motor y accesorios |
| 12465 | Moda y accesorios |
| 200 | Inmobiliaria |
| 24200 | Tecnología y electrónica |
| 12579 | Deporte y ocio |
| 17000 | Bicicletas |
| 12467 | Hogar y jardín |
| 13100 | Electrodomésticos |
| 12463 | Cine, libros y música |
| 12461 | Niños y bebés |
| 18000 | Coleccionismo |
| 19000 | Construcción y reformas |
| 20000 | Industria y agricultura |
| 21000 | Empleo |
| 13200 | Servicios |
| 12485 | Otros |

> Categorías sin subcategorías: 100, 14000, 200, 12485.
> Nota: `config.CATEGORIES` (tabla offline de nombres de la CLI) se corrigió
> con estos ids reales — los antiguos (12545, 16000, 15000, 12900…) estaban
> obsoletos y faltaban 12800, 24200 y 12485.

### Verificación de subcategorías (Playwright)

Haciendo clic en el árbol lateral se abre **un submenú por cada nivel** y cada
selección genera una URL con `subcategory_ids=<id>` **igual al `id` del árbol de
la API**. Ejemplo real capturado — categoría `12800` (Motor y accesorios):

| Subcategoría del menú (EN) | `subcategory_ids` | Nodo API (ES) |
|---|---|---|
| GPS & electronics | `10164` | GPS y electrónica |
| Car & van spare parts | `10167` | Recambios de coches y furgonetas |
| Mechanical tools | `10165` | Herramientas |
| Motorbike and four-wheeler spare parts | `10168` | Recambios de motos y cuatriciclos |
| Others | `10166` | Otros |

Y a **dos niveles** — categoría `24200`, subcategoría `Sound` → `Headphones`
(`24149` → `10395`):

```
...search?category_id=24200&subcategory_ids=24149   # Sound
...search?category_id=24200&subcategory_ids=10395   # Headphones (nieto)
```

Ambos coinciden con los ids del árbol recursivo de `/categories`. La brújula
práctica: **un `name` del menú lateral = el `name` localizado de un nodo del
árbol con el mismo `id`**. Con esto, `--subcat` de la CLI y los `subcategory_ids`
de cualquier URL de la web apuntan al mismo árbol.

### GET `/api/v3/general/search` (legacy)

Alternativa antigua si `/search/components` no devuelve `search_id`.

- Params: `keywords`, `order_by`, `filters_source`, `source`, opcional
  `category_ids` (plural), `subcategory_ids`, `condition`, `min_sale_price`,
  `max_sale_price`, `is_shippable=true`, `latitude`, `longitude`,
  `distance=1000000`.
- Respuesta: `search_objects[]`.
- Código: `Wallapop._search_legacy()`.

### Comportamiento geográfico (cómo decide el país/alcance)

Esto es lo que descubrimos reproduciendo el flujo del navegador real
(`es.wallapop.com/search`), e importa para quien llame a `search()`:

- **Sin `latitude`/`longitude` el API devuelve 0 resultados.** El backend no
  tiene default: necesita un ancla geográfica para decidir qué mostrar. La web
  "no lo nota" porque el navegador **geolocaliza por IP/sesión** y manda
  coords + `search_country` siempre — no es que la web no use coords, es que
  las calcula por ti.
- **`search_country` es el que más manda.** El país devuelto en los items
  depende sobre todo de este param (+ coords). Verificado:
  - `coords PL + search_country=PL` → 0 items (no hay gamecubes anclados a PL).
  - `coords PL + search_country=ES` → 40 items **de España** (el país domina
    sobre las coords).
  - `coords ES + search_country=ES` → 40 items (39 ES + 1 PT de frontera).
- **El país casi no filtra los resultados por contenido**, solo influye en el
  orden (con `order_by=closest`) y en flags de envío/moneda. Con
  `order_by=most_relevance` los items salen del país de `search_country`.
- **`order_by=newest` ignora el país** y devuelve un conjunto más global
  (p. ej. mezcla de España + otros países), igual que pasa en la app cuando "a
  veces te saca cosas de Italia o Portugal".
- **`distance` no amplía el alcance** en el flujo `/search/components` +
  `/search/section` (probado con 1M y 50M: mismo resultado). El legacy
  `/general/search` acepta `distance` pero hoy devuelve 0 items.

### Default de la librería

`Wallapop.search()` usa **España por defecto** (`CITIES["espana"]` = 40.0,
-3.5) cuando no se pasan `latitude`/`longitude`. Verificado que devuelve
**exactamente** los mismos items (mismos ids y orden) que `search_country=ES`
con las coords españolas — es decir, reproduce la búsqueda que verías en ese
país en `es.wallapop.com`. No es una restricción: si se pasan coords de otro
país (p. ej. `CITIES["italia"]`), la búsqueda devuelve los items de ese país.

---

## Anuncios

### GET `/api/v3/items/{id}`

Detalle público de un anuncio (sin auth).

- Respuesta: JSON plano del item (precio con su estructura, `user`,
  `web_slug`, etc.).
- `images`: array de `{id, average_color, urls: {small, medium, big}}` con
  URLs a `cdn.wallapop.com` (cada size a un ancho: W320 / W640 / W800).
  `Item.images` guarda las URLs `medium` (W640).
- Código: `Wallapop.item()`, `Wallapop.download_images()` (también se usa
  `item()` para averiguar la moneda al mandar una oferta).

### GET `https://es.wallapop.com/item/{slug}`  [página]

Resolución de referencia → id interno. La página Next.js embebe el primer
payload en `<script id="__NEXT_DATA__" type="application/json">`.

- Extrae `data.props.pageProps.item.id` (el slug literal puede contener un
  sufijo que no coincide con el id).
- Código: `resolve_internal_id()` (aplica también si ya recibe un url o un
  hash de 12 caracteres alfanuméricos).

---

## Usuarios y valoraciones

### GET `/api/v3/users/{user_id}`

Perfil público del vendedor (sin auth).

- Puntos clave: `micro_name` (nombre), `is_top_profile`, `location`,
  `register_date`, `seller_type`.
- NO trae la valoración (estrellas/reseñas) — eso va en `/stats` (media y
  total) y `/reviews` (detalle).
- Código: `Wallapop.seller()`; lo usa el comando CLI `seller` (nombre).

### GET `/api/v3/users/{user_id}/reviews`

Reseñas detalladas del vendedor (sin auth).

- Respuesta: array (una por transacción). Cada ítem tiene
  `review.rating_over_five` (estrellas 1-5), `review.scoring` (0-100),
  `review.comments` (texto) y `item.title` (qué vendió).
- El array está paginado y no expone el total exacto; para el promedio y el
  total reales usa `/stats`.
- Código: `Wallapop.seller_reviews()`; lo usa el comando CLI `seller`
  (reseñas detalladas).

### GET `/api/v3/users/{user_id}/stats`

Estadísticas agregadas del vendedor (sin auth) — **los valores que la web
muestra en la página del item** (p. ej. "★ 4.9 · 465 reseñas").

- `rating_average`: media de estrellas (p. ej. `4.9`).
- `counters[]`: lista de `{type, value}` incluyendo `reviews` (total de
  reseñas), `publish`, `buys`, `sells`, `sold`.
- Además `shipping`, `deliveries_on_time_in_days`.
- Código: `Wallapop.seller_stats()`; lo usan `show` (media + nº de reseñas en
  la línea del vendedor) y el comando CLI `seller` (valoración + stats).

---

## Mensajería

### POST `/api/v3/instant-messaging/conversation`

Crea (o devuelve) la conversación de un anuncio. Idempotente.

- Body: `{"item_hash_id": <item hash>}`
- De la respuesta se extrae `conversation_id`.
- Código: `Wallapop.ensure_conversation()`.

### GET `/bff/messaging/conversation/{conversation_id}`  [BFF]

Hilo completo de una conversación.

- Respuesta: `with_user`, `item`, `channel`, `sale`, y
  `messages.messages[]` (texto o payload de "delivery" para ofertas).
- Nota: los mensajes llegan newest-first; hay que ordenar por `timestamp`.
- Código: `Wallapop.conversation_thread()`.

### GET `/bff/messaging/inbox?page_size=30&max_messages=30`  [BFF]

Inbox; por conversación incluye sus N últimos mensajes (newest-first) y el
`item` asociado.

- Código: `Wallapop.inbox()`.

### GET `/api/v3/instant-messaging/messages/unread`

Contador de no leídos (`unread_counter`).

- Código: `Wallapop.unread_count()`.

### GET `/api/v3/instant-messaging/token`

Token de PubNub (se usa como `auth` en el publish).

- Respuesta: `data.token`.
- Código: `Wallapop.im_token()`.

### GET `https://ps7.pndsn.com/publish/{pub}/.../{channel}/0/{msg}`  [PubNub REST]

Entrega real de un mensaje de chat (así lo hace la web).

- Ruta: `publish/{PUB_KEY}/{SUB_KEY}/0/{channel}/0/{json_url_encoded}`.
- Params: `meta` (JSON con tipo de mensaje, `to_user_hash`,
  `from_user_hash` [= `hashed_sub` del JWT], `conversation_hash`,
  platform), `uuid` (= hash de usuario), `requestid`, `pnsdk`, `auth`.
- Ack de éxito: `[1,"Sent",<timetoken>]`.
- Código: `Wallapop.pubnub_publish()`.

---

## Ofertas

### POST `/api/v3/delivery/buyer/offers`

Enviar una oferta (flujo actual de la web; el viejo `/items/{id}/offers` está
muerto: 404).

- Body: `{"offer_id": <uuid generado por el cliente>, "offer_price_amount":
  <float>, "offer_price_currency": "EUR", "item_ids": [<item id>],
  "message": "..." (opcional)}`.
- Si manda falla `400/422` y había mensaje, se reintenta sin él.
- Código: `Wallapop.send_offer()`.

### PATCH `/api/v3/delivery/offers/{offer_id}/status`

Aceptar o rechazar una oferta (p. ej. la contraoferta de un vendedor).

- Body: `{"status": "ACCEPTED" | "DECLINED"}`.
- Comportamiento real verificado: sobre una oferta ya cerrada devuelve
  `409 invalid offer status` (validación del servicio, no bloqueo anti-bot).
- Tras aceptar, el hilo recibe *"Oferta aceptada, ¡cómpralo ahora! Esta
  oferta caduca en 24h."*; al rechazar, *"Has rechazado la oferta."*.
- Código: `Wallapop.offer_response()`.

### PATCH `/api/v3/delivery/offers/{offer_id}/offer-price`

Contraofertar un precio sobre una oferta abierta (flujo "hacer una nueva
oferta" de la web).

- Body: `{"offer_price_amount": <float>, "offer_price_currency": "EUR"}`.
- Código: `Wallapop.counter_offer()`.

### GET `/bff/delivery/offer-details?offer_id=`  [BFF]

Payload del modal "Offer Tracking Status".

- Solo tiene sentido cuando **nosotros somos el que debe responder** (el
  payload se genera para el respondiente actual).
- ⚠️ **Bloqueado por anti-bot para `urllib`**: devuelve 302 →
  `/notAvailable`, y con cabeceras `Sec-Fetch-*`/`mpid`/`trackinguserid`
  pasa a 200 pero con el shell HTML de la SPA (no JSON). En navegador
  (Playwright + cookies de sesión) sí devuelve el JSON.
- La CLI no depende de él: reconstruye el estado desde el hilo de chat
  (`parse_offer_events`). Se documenta por integridad y por si la entrega
  cambia.
- Código: `Wallapop.offer_details()` (actualmente solo diagnóstico).

### GET `/bff/delivery/make-an-offer?item_id=`  [BFF]

Config del editor de ofertas.

- Respuesta: `item.price.amount`, y dentro de `restriction`:
  `max_offer_percentage` (límite de descuento) y
  `remaining_offers_per_day`.
- Código: `Wallapop.make_offer_info()`.

---

## Modelo del ledger de ofertas (sin API extra)

El estado de una negociación se deriva **offline** de los mensajes del hilo:

- El mensaje de oferta llega como `type=delivery_generic` con un `payload` cuyo
  `text` (o `buttons[].action` con `app/chat/offer/<uuid>`) identifica el
  evento; tipos: `our-offer`, `seller-counter`, `our-counter`,
  `seller-accepted`, `accepted-buy`, `seller-declined`, `our-declined`,
  `expired`.
- Los mensajes terminales (aceptado/rechazado/caducado) no traen importe:
  se hace *forward-fill* del `amount`/`offer_id` desde el último evento
  anterior con valor.
- Código: `parse_offer_events()` + `offer_state_summary()` (`models.py`).

---

## Publicar artículos (upload / publish)

> Estado: flujo completo mapeado y publicación funcionando por **API pura**
> (`wallapop publish`, sin navegador) y también **vía navegador**
> (`wallapop publish --mode browser`, módulo `wallapoplib/publish.py`).
> El body multipart del `POST /api/v3/items` se capturó con
> `page.route` + `request.post_data_buffer` (el `post_data` de CDP era `null`,
> pero el buffer binario sí se expone) — ese es el plano que replica el método
> `Wallapop.publish()` (ver "Paso final: publicar").

### POST `/api/v3/steps`  [state machine]

Motor del wizard de publicación. Máquina de estados sin estado: cada petición
envía el borrador completo y devuelve el siguiente `step.id` a resolver.

**Headers comunes**: `Authorization: Bearer <token>`, `Content-Type: application/json`,
`Origin: https://es.wallapop.com`, `X-AppVersion: 826590`, `X-DeviceOS: 0`,
`X-DeviceID: <device_id>`.

#### 1. Init → title

```http
POST /api/v3/steps
{"mode": {"action": "upload", "id": "<upload_id>"}}
```

- `upload_id` = UUID generado por el cliente (identifica el borrador).
- Respuesta: `{"step": {"id": "title", "components": [], "metadata": {}}}`.

#### 2. title → photo

```http
POST /api/v3/steps
{
  "mode": {"action": "upload", "id": "<upload_id>"},
  "current_step": "title",
  "draft": {"summary": "Nintendo 3DS XL Azul Yveltal Edicion Pokemon"}
}
```

- `summary` = título (máx 50 chars, valida: sin enlaces, no solo espacios, no emojis).
- Respuesta: `step.id: "photo"` con `picture_upload.path` = `/api/v3/upload/<upload_id>/pictures`,
  `max_number_of_pictures: 1` (una foto por llamada), `timeout_ms: 20000`, `max_size_px: 768`,
  y `pictures_item.max_number_of_pictures: 10` (máx 10 fotos por anuncio).

#### 3. photo → category

```http
POST /api/v3/steps
{
  "mode": {"action": "upload", "id": "<upload_id>"},
  "current_step": "photo",
  "draft": {"summary": "Nintendo 3DS XL Azul Yveltal Edicion Pokemon"}
}
```

- El body usa el **mismo draft** que title (no se mandan IDs de foto; la subida es aparte).
- Respuesta: `step.id: "category"` con `components: [{"id": "category", "type": "custom", ...}]`.

#### 4. category → listing

```http
POST /api/v3/steps
{
  "mode": {"action": "upload", "id": "<upload_id>"},
  "current_step": "category",
  "draft": {"category_leaf_id": "10088", "root_category_id": "24200"}
}
```

- **`category_leaf_id` y `root_category_id` son STRINGS** ("10088", "24200"), no enteros.
- `category_leaf_id` = hoja (ej. 10088 = Consolas), `root_category_id` = raíz (24200 = Tecnología y electrónica).
- Respuesta: `step.id: "listing"`, `components: []`.

#### 5. listing → (shipping → publish)

El step `listing` es el formulario de atributos/precio/descripción/estado/envío.
La estructura del draft validada:

```json
{
  "summary": "Nintendo 3DS XL Azul Yveltal Edicion Pokemon",
  "description": "Nintendo 3DS XL en buen estado. Edicion Pokemon Yveltal azul. Con cargador. Envio disponible.",
  "price_amount": "400",
  "category_id": "10088",
  "brand": "Nintendo",
  "model": "Nintendo 3DS XL",
  "condition": "good",
  "pictures": []
}
```

**Campos críticos:**
- `price_amount` = **string** "400" (no int, no objeto `price: {amount, currency}`).
- `category_id` = string "10088".
- `condition` = string "good" (valores: `un_opened`, `in_box`, `new`, `as_good_as_new`, `good`, `fair`, `has_given_it_all`).
- `color` = string "blue" (no array).
- `brand`, `model` = strings opcionales.
- `description` = string (máx 640).

Enviar `draft` con todos los campos → 200 pero **no avanza** (siguiente step sigue siendo `listing`).

### Subida de fotos: `POST /api/v3/upload/<upload_id>/pictures`

Una llamada por foto (máx 10). Multipart `file` (image/jpeg).
- Respuesta: **204 No Content** (sin IDs de foto en respuesta ni headers).
- Límite: `pictures_item.max_number_of_pictures: 10` (la carpeta puede tener más, solo suben 10).

### POST `/api/v3/items/upload/components`  [BDUI form schema]

Sirve el esquema (components) del formulario `listing` para el borrador actual.
Se invoca cada vez que cambia un campo con `reloads_when_changed: true` (ej. `condition`).

Request real (**formato confirmado desde el navegador**):
```json
{
  "fields": {
    "summary": "Nintendo 3DS Azul Cobalto",
    "category_leaf_id": "10088",
    "root_category_id": "24200",
    "brand": "Nintendo",
    "model": "3DS",
    "color": "blue",
    "title": "Nintendo 3DS Azul Cobalto",
    "description": "Consola Nintendo 3DS en color Azul Cobalto...",
    "condition": "good",
    "price_amount": "310"
  },
  "mode": {"action": "upload", "id": "<upload_id>"}
}
```

- El cuerpo va en **`fields`** + `mode` (misma estructura que `/steps`).
- Todos los valores son **strings** (`category_leaf_id` "10088", `price_amount` "310").
- Se hace un primer POST solo con `summary`/`category` al llegar a `listing`, y
  un segundo POST **con todos los campos** (precio, condición, marca, modelo…)
  justo antes de publicar. Ambos devuelven `200`.
- Marca/modelo/color/título/descripción se autocompletan solos tras subir las
  fotos (suggested-item-data); el usuario solo edita precio + condición + envío.

Respuesta 200 con array de components (`text_input`, `combo_box`, `dropdown`, `custom`, `feature_banner`).
Campos observados en orden:
1. `suggested_data_banner` (feature_banner, no editable)
2. `photo` (custom, `max_number_of_pictures: 10`)
3. `brand` (combo_box, requerido, opciones via `/api/v3/attributes/brand/options?category_leaf_id=...`)
4. `model` (combo_box, opcional, opciones via `/api/v3/attributes/model/options?brand=...&category_leaf_id=...`)
5. `color` (dropdown, opcional)
6. `title` (text_input, requerido, max_length: 50, valida: sin enlaces)
7. `description` (text_area, requerido, max_length: 640)
8. `condition` (dropdown, requerido, `reloads_when_changed: true`)
9. `price_amount` (text_input, requerido, rango envío 1–2500 €)
10. `lock_stocks` (lock, opcional, "Añadir más unidades", `root_category_id: 24200`)

### Config de envío (consumida por el formulario listing)

**GET `/api/v3/delivery/weight/tiers/with-suggestion`**
```json
{"weight_tiers": [
  {"min_weight_in_kg": 0, "max_weight_in_kg": 1, "min_cost": {"amount": 2.99, "currency": "EUR"}, "max_cost": {"amount": 6.49, "currency": "EUR"}, "suggested": false},
  ...
]}
```

**GET `/bff/delivery/rules`**
```json
{"categories_with_shipping_not_allowed": [100, 14000, ...],
 "subcategories_with_shipping_not_allowed": [10119, 10120, ...],
 "price_range_allowed": {"min_price": 1.0, "max_price": 2500.0}}
```

### Paso final: publicar

El POST que crea el anuncio definitivo (confirmado con publicaciones reales):

```http
POST /api/v3/items
```

**Verificado en vivo**: 200 y crea el item (hemos publicado artículos reales). Tras el
POST la web hace `GET /api/v3/items/<id>?language=es` (el nuevo id) y sube las fotos
finales con `POST /api/v3/items/<id>/picture2` (una por foto, todas 200), luego
redirige a `/app/catalog/published`.

**Headers reales del POST** (captura CDP del navegador):
- Preflight OPTIONS pide `Access-Control-Request-Headers: authorization,x-deviceos`.
- El POST manda `Authorization: Bearer <token>`, `Content-Type: multipart/form-data;
  boundary=...`, `Accept: application/vnd.upload-v2+json`, `x-deviceos: 0`,
  `Origin/Referer: https://es.wallapop.com`.

**Body multipart completo** (capturado con `page.route('**/api/v3/items')` +
`request.post_data_buffer`, sin el decode que rompía con un JPEG; 966 B sin foto):

```
------WebKitFormBoundary<...>
Content-Disposition: form-data; name="image"; filename="foto.jpg"
Content-Type: image/jpeg

<bytes de la foto>
------WebKitFormBoundary<...>
Content-Disposition: form-data; name="item"

{"attributes":{"suggested_data_banner":null,"brand":"Nintendo","model":"3DS","color":"blue",
  "title":"Nintendo 3DS Azul Cobalto","description":"...","condition":"good",
  "price_amount":330},"category_leaf_id":"10088","apply_discount":false,
  "location":{"latitude":43.66844411761024,"longitude":-7.605464501519192,"approximated":false},
  "delivery":{"allowed_by_user":true,"max_weight_kg":1,
    "cost_configuration_id":"814429d6-..."},"upload_id":"<uuid>"}
------WebKitFormBoundary<...>--
```

Solo dos campos: **`image`** (la foto, filename genérico) y **`item`** (JSON):
- `attributes.price_amount` es un **entero** ("330" sin comillas), a diferencia del
  `"310"` (string) del esquema `components`.
- `location` = lat/lon **no aproximados** del vendedor, que se obtienen de
  `GET /api/v3/users/me` (`location.approximated_latitude`, `.approximated_longitude`,
  `.approximated_location`).
- `delivery.cost_configuration_id` es un **UUID constante por cuenta** (el mismo que
  devuelve `shipping.cost_configuration_id` de cualquier item publicado del usuario).
- `upload_id` = UUID generado por el cliente (el mismo `mode.upload.id`).
- Respuesta 200: `{"id":"<item id>","flags":{}}` — **el id del anuncio viene en la
  respuesta del POST** (no hay que hacer GET posterior para conocerlo).

**Implementación — API pura sin navegador**: `Wallapop.publish()` (`client.py`)
replica exactamente ese multipart (foto + JSON `item`) y publica con HTTP plano.
Rellena `location` con `GET /users/me`, `cost_configuration_id` con el de cualquier
item publicado de la cuenta (fallback a un valor conocido) y `upload_id` con un UUID
fresco. Verificado en vivo: 200 `{"id":...}`, el anuncio aparece publicado y con la
foto. Es el modo por defecto de `wallapop publish`; `--mode browser` conserva el
wizard Playwright.

El anuncio creado (estructura de un item publicado) revela:
- `price: {cash: {amount, currency: "EUR"}}`
- `shipping: {item_is_shippable: true, user_allows_shipping: true, cost_configuration_id: "..."}`
- `location: {lat, lon, city, postal_code}`
- `type_attributes` incluye `up_to_kg: "1.0"` (tramo de peso elegido)
- `condition`, `brand`, `model`, `color` (los mismos del formulario)

Endpoints de referencia en bundle JS:
- `chunk-B5DKKJDO.js` — payload builder `ce` → POST `/api/v3/items`
- `chunk-IQHJVPZ3.js` — wizard steps (`POST /api/v3/steps`, `suggested-item-data`, constantes `SUMMARY, CATEGORY, PHOTO, LOADING, LISTING, ISBN`)
- `chunk-3WKBKU6R.js` — `api/v3/items/upload/components`

## Editar un anuncio publicado

### PUT `/api/v3/items/{id}`  [editar/actualizar]

Edita un anuncio ya publicado (lo que hace la web al pulsar "Actualizar" en el
editor). Capturado con `page.on("request")` durante una edición real (2026-09-06).

**Verificado en vivo**: todos los atributos editables devuelven **204** y el
cambio se refleja al hacer `GET /items/<id>`.

**Headers reales del PUT** (imprescindibles — sin ellos da 404 `null for uri`):
```http
PUT /api/v3/items/<id>
Accept: application/vnd.upload-v2+json       # el MÁS importante
Accept-Language: es,es-ES;q=0.9
Authorization: Bearer <token>
Content-Type: application/json
Origin: https://es.wallapop.com
Referer: https://es.wallapop.com/
User-Agent: Mozilla/5.0
X-AppVersion: 826800
X-DeviceID: <device_id>
X-DeviceOS: 0
mpid: <mpid>
trackinguserid: <mpid>
```
- `PATCH /items/<id>` y el mismo PUT **sin** `Accept: application/vnd.upload-v2+json`
  → `404 null for uri` (no es CORS ni auth: es el header que selecciona el handler).
- `pictures[].id` son los ids de las fotos del item (de `GET /items/<id>` → `images[].id`),
  no el id del item.

**Body JSON completo** (la web manda SIEMPRE todos los campos, con cada atributo
propuesto en el form):
```json
{
  "attributes": {
    "brand": "Nintendo",
    "model": "3DS",
    "color": "blue",
    "title": "Nintendo 3DS Azul Cobalto",
    "description": "Consola en buen estado...",   // STRING plano, no el dict {original}
    "condition": "good",
    "price_amount": 520                            // entero
  },
  "category_leaf_id": "10088",
  "apply_discount": false,
  "location": {"latitude": 43.66, "longitude": -7.60, "approximated": false},
  "delivery": {"allowed_by_user": true, "max_weight_kg": 1,
               "cost_configuration_id": "<uuid del item>"},
  "pictures": [{"id": "<foto_id>", "order": 0}]
}
```
- `description` debe ir como **string plano**. `GET /items/<id>` la devuelve como
  `{"original": "..."}`; si reenvías ese dict, el backend responde `400`.
- `location` y `cost_configuration_id` se toman de `GET /users/me` y del item.

**Atributos verificados editables uno por uno (todos 204, 2026-09-06)**:
`brand`, `model`, `color`, `title`, `description`, `condition`, `price_amount`,
`max_weight_kg`, `category_leaf_id` (mueve el anuncio de categoría). El resto del
body debe ir como estaba (la web reenvía el formulario completo).

### Añadir imágenes y cambiar la principal

- **Añadir**: `POST /api/v3/items/<id>/picture2` (multipart `image` = bytes JPEG
  completos + `order` como string) → 200 con el **id de la foto nueva**. El item
  gana una imagen por llamada. (Misma receta que para publicar multi-foto; ver
  `docs/multi-photo-upload.md`.)
- **Cambiar la principal / reordenar**: `PUT /api/v3/items/<id>` con
  `pictures` listando las fotos en el orden deseado: `order: 0` = principal.
  La foto que se omite deja de aparecer en el anuncio (verificado en vivo:
  sustituir la imagen de `p618gorvp5z5` y añadir 2 más, quedando 3 fotos con la
  nueva como portada).

### Reservar un artículo: `PUT /api/v3/items/<id>/reserve`

```http
PUT /api/v3/items/<id>/reserve
Content-Type: application/json
Body: {"reserved": true}     # para desreservar: {"reserved": false}
```

- Devuelve **204**. Se verifica con `GET /items/<id>` → el campo `reserved`
  pasa a `{"flag": true}` o queda `null` al desreservar.
- Validado por API pura con los headers base del CLI (2026-09-06): publicado
  un item de prueba y marcado reservado con este PUT → 204. Confirmado además
  que el botón "Reservar" de la web vendedor manda exactamente el mismo.
- No exige el `Accept: application/vnd.upload-v2+json` del PUT de edición.

### Marcar un artículo como vendido: `PUT /api/v3/items/<id>/sold`

```http
PUT /api/v3/items/<id>/sold
Content-Type: application/json
Body: {}                                  # vacío
```

- Devuelve **204** sin body. Verificado con `GET /items/<id>` → el campo
  `sold` pasa a `{"flag": true}`, tanto por el flujo de la web como por API
  pura (`PUT /items/<id>/sold` con `body={}` y headers base del CLI).
- Capturado en vivo desde la web vendedor: el botón "Marcar como vendido"
  abre un modal y el confirm final dispara justo este `PUT /items/<id>/sold`
  (con la cookie de sesión; mismo request que manda la API con auth bearer).
- Es distinto de `reserve`: aquí el body es vacío y el flag resultante es
  `sold`, no `reserved`. No se capturó des-marcar (no hay botón de
  "desvender" en la web).
- Verificado en tres anuncios de prueba (2026-09-06): `p618gorvp5z5`,
  `8z8871mrqdz3` y `8z8871mpkdz3` (web) y `mzn3nlyqq2jn` (API pura,
  vendido además con `body={}` quedando `sold: {"flag": true}`).

### Marcar/desmarcar un artículo como favorito: `PUT /api/v3/items/{id}/favorite`

```http
PUT /api/v3/items/{id}/favorite
Content-Type: application/json
Body: {"favorited": true}     # para desmarcar: {"favorited": false}
```

- Devuelve **204** sin body. Se verifica con `GET /items/{id>` → el campo
  `favorited` pasa a `{"flag": true}` (o queda `false`/`null` al desmarcar).
- Capturado desde la web (HAR): el botón de corazón en la página del item
  dispara este PUT con body `{"favorited": true}`.
- Los headers necesarios son los base del CLI (`Authorization`, `Content-Type`,
  `Origin`, `X-DeviceID`, `X-AppVersion`, `X-DeviceOS`). No necesita el header
  `Accept: application/vnd.upload-v2+json` del PUT de edición.
- El bundle JS de la web define la ruta como `api/v3/items/{id}/favorite`
  (chunk 23852, exportado como `itemFavoriteUrl`).
- Código: `Wallapop.favorite(item_id, favorited)` (client.py).

### Borrar un artículo: `DELETE /api/v3/items/{id}`

```http
DELETE /api/v3/items/{id}
```

- Devuelve **204**. Borrado permanente: elimina el anuncio del marketplace y
  deja de existir (un GET posterior devuelve 404). Es distinto de
  `PUT /items/{id}/reserve` (lo mantiene visible pero reservado), de
  `PUT /items/{id}/sold` (lo quita del marketplace pero lo conserva en el
  historial de ventas), y de `PUT /items/{id}/inactivate` (lo oculta
  temporalmente).
- Verificado en vivo (2026-09-23): publicar un item de prueba → 200, borrarlo
  con `DELETE /items/{id}` → 204, y `GET /items/{id}` del mismo → 404.
- Los headers necesarios son los base del CLI (`Authorization`, etc.), sin el
  `Accept: application/vnd.upload-v2+json` de la edición. También hay que
  pasar el `item_id` interno (hash de 12 chars de la dirección `#/item/...`),
  igual que el resto de endpoints de items.
- Documentado también en la API pública de Wallapop para desarrolladores
  (`developers.wallapop.com`, método oficial `deleteItem` sobre el mismo
  recurso `DELETE /items/{itemId}`).
- Código: `Wallapop.delete_item(item_id)` (client.py), comando CLI
  `wallapop delete <item> [--yes]` (commands.py).

---

## Centro de compras y ventas (SDUI)

Las páginas **Compras** (`/app/purchases?selectedTab=...`) y **Ventas**
(`/app/selling?selectedTab=...`) de la web son *server-driven UI*: el endpoint
devuelve un árbol de widgets y el frontend lo pinta tal cual.

### GET `/sdui/purchases/{ongoing|finished}`  [root, auth]
### GET `/sdui/selling/{ongoing|finished}`   [root, auth]

- **Viven en la raíz del host** (`https://api.wallapop.com/sdui/...`), **sin**
  `/api/v3` — como los `bff`. Con `/api/v3` delante devuelven
  `404 null for uri .../api/v3/sdui/...`.
- Requieren `Authorization: Bearer`. Sin params de query extra (capturado en
  vivo del navegador): GET plano con las cabeceras base del CLI.
- Estructura de la respuesta (widget tree):
  - `main.data.content[0]` → `tab_container` con las dos `tabs`
    (`id: ongoing`/`finished`, `label` localizado según `Accept-Language`) y
    `content` (spacer + `directory`).
  - `directory.data.content[]` es la lista: nodos `text` (cabeceras de
    año/mes), `list_item_transaction` (las tarjetas de pedido) o
    `empty_state` (pestaña vacía, con título/subtítulo/botón).
- Cada `list_item_transaction`:
  - `id` (uuid del pedido), aunque el enlace real va en `events.on_click.data.url`
    (tracking `app/delivery/tracking/<uuid>` para compras con envío, o la URL
    del item en las ventas vendidas en persona).
  - `data.main.data.content` (título del artículo),
    `data.detail.data.content` (p. ej. "Transacción completada 25 abr 2025"),
    `data.detail_price.data.content` ("14,88 €").
  - `data.extra_details[]`: detalles `list_item_transaction_plain_detail`
    (label/valor: fecha de pedido, código de envío, método de entrega) y
    `list_item_transaction_contact_detail` (avatar + `label` = la otra parte).
  - `data.avatar.data.main_image_url` (foto del producto).
- La CLI **no pinta el árbol**: `parse_sdui_transactions()` (`models.py`)
  aplana solo las tarjetas (`id`, `title`, `detail`, `price`, `details[]`,
  `url`), descartando cabeceras de agrupación y `empty_state`.
- Código: `Wallapop.purchases(tab)`, `Wallapop.selling(tab)` (client.py) y los
  comandos CLI `wallapop purchases [--finished]` / `wallapop selling [--finished]`.

## Cobertura del servidor MCP

Este catálogo conserva la investigación del proyecto original. La superficie actual del servidor Rust y sus límites se documentan en [MCP.md](MCP.md). Los endpoints no oficiales pueden cambiar; las pruebas locales no verifican su disponibilidad en vivo.
