# WoW 3.3.5a (Build 12340) — Memory Offsets Reference

> All адреса относительны к базовому адресу `Wow.exe` в памяти (image base).
> Стандартный image base для Wow.exe 3.3.5a — `0x00400000`, но все указанные ниже адреса
> уже являются абсолютными виртуальными адресами (VA), как их видит процесс.

---

## 1. Chat Buffer (Кольцевой буфер чата)

WoW хранит последние 60 сообщений чата в кольцевом (circular) буфере фиксированного размера.
Когда буфер заполняется (индекс достигает 59), следующее сообщение записывается в слот 0,
перезаписывая самое старое.

### 1.1 Ключевые адреса

| Имя | Адрес | Тип | Описание |
|---|---|---|---|
| `ChatBufferStart` | `0x00B75A58` | — | Базовый адрес массива из 60 структур сообщений |
| `ChatMessageStride` | `0x17C0` | const | Размер одной структуры сообщения (6080 байт). Это **не указатель**, а константа |
| `ChatBufferCount` | `0x00BCEFEC` | uint32 | Индекс **следующего** слота для записи (0–59). Инкрементируется при каждом новом сообщении, оборачивается через `% 60` |

> **Примечание:** В некоторых проектах (AmeisenBot) используется адрес `0x00B75A60` (+8 байт)
> как начало буфера — это пропуск поля `SenderGuid` первой записи. Каноничный старт — `0x00B75A58`.

### 1.2 Формула адреса сообщения

```
message_address = ChatBufferStart + (index * ChatMessageStride)
message_address = 0x00B75A58 + (index * 0x17C0)
```

Где `index` — число от 0 до 59.

### 1.3 Алгоритм чтения новых сообщений

```
1. При старте: прочитать ChatBufferCount → сохранить как last_index
2. В цикле (каждые 100–500 мс):
   a. Прочитать текущий ChatBufferCount → current_index
   b. Если current_index != last_index:
      - Прочитать все сообщения от last_index до current_index
        (учитывая оборачивание через 60)
      - Обновить last_index = current_index
3. Оборачивание: если current_index < last_index,
   читаем [last_index..60) и затем [0..current_index)
```

---

## 2. Chat Message Structure (Структура сообщения)

Каждое сообщение занимает ровно `0x17C0` (6080) байт. Ниже — раскладка полей
с двумя вариантами (классическим и расширенным).

### 2.1 Вариант 1: Классическая раскладка

Источник: OwnedCore — [C# Chat Buffer Memory (Updated)](https://www.ownedcore.com/forums/world-of-warcraft/world-of-warcraft-bots-programs/wow-memory-editing/280764-c-copypasta-chat-buffer-memory-updated.html)

| Поле | Смещение (hex) | Смещение (dec) | Размер | Тип | Описание |
|---|---|---|---|---|---|
| SenderGuid | `0x0000` | 0 | 8 | uint64 | GUID отправителя сообщения |
| Unknown | `0x0008` | 8 | 52 | uint32[13] | Неизвестные поля / padding |
| FormattedMessage | `0x003C` | 60 | 3000 | char[] | Отформатированное сообщение (с цветовыми кодами, ссылками и т.д.) |
| PlainText | `0x0BF4` | 3060 | 3000 | char[] | Чистый текст сообщения (без форматирования) |
| MessageType | `0x17AC` | 6060 | 4 | uint32 | Тип сообщения (см. таблицу ниже) |
| ChannelNumber | `0x17B0` | 6064 | 4 | uint32 | Номер канала (для сообщений типа CHANNEL) |
| Sequence | `0x17B4` | 6068 | 4 | uint32 | Порядковый номер сообщения |
| Timestamp | `0x17B8` | 6072 | 4 | uint32 | Время сообщения (game time) |
| _(padding)_ | `0x17BC` | 6076 | 4 | — | Выравнивание до stride 0x17C0 |
| **Итого** | | | **6080** | | = `0x17C0` |

### 2.2 Вариант 2: Расширенная раскладка (с SenderName)

Источник: тот же тред, обновленная версия.

В области `Unknown` (смещение `0x0008`–`0x003B`) часть данных содержит имя отправителя:

| Поле | Смещение (hex) | Размер | Тип | Описание |
|---|---|---|---|---|
| SenderGuid | `0x0000` | 8 | uint64 | GUID отправителя |
| Unknown1 | `0x0008` | 4 | uint32 | |
| Unknown2 | `0x000C` | 4 | uint32 | |
| Unknown3 | `0x0010` | 4 | uint32 | |
| Unknown4 | `0x0014` | 4 | uint32 | |
| SenderName | `0x0018` | 49 | char[49] | Имя отправителя (null-terminated, макс 48 символов + `\0`) |
| _(gap)_ | `0x0049` | — | — | Переход к FormattedMessage |
| FormattedMessage | `0x003C`* | 3000 | char[] | _*Точное смещение может варьироваться_ |

> **Примечание:** Вариант 2 не полностью верифицирован — SenderName в смещении `0x0018`
> перекрывается с FormattedMessage в `0x003C`. Рекомендуется использовать Вариант 1
> и извлекать имя отправителя из `FormattedMessage` или через `NameStore` по GUID.

### 2.3 Строки — null-terminated

Оба строковых поля (`FormattedMessage`, `PlainText`) — это null-terminated ASCII/UTF-8.
Нужно читать до первого `\0` или до максимальной длины 3000 байт.

---

## 3. Message Types (Типы сообщений)

Поле `MessageType` (`0x17AC`) содержит числовой идентификатор типа чата:

| Значение | Имя | Описание |
|---|---|---|
| 0 | `ADDON` | Аддон-сообщение (скрытое) |
| 1 | `SAY` | /say — сообщение в радиусе слышимости |
| 2 | `PARTY` | /party — групповой чат |
| 3 | `RAID` | /raid — рейдовый чат |
| 4 | `GUILD` | /guild — гильдийный чат |
| 5 | `OFFICER` | /officer — офицерский чат гильдии |
| 6 | `YELL` | /yell — крик (большой радиус) |
| 7 | `WHISPER` | /whisper — входящий шепот |
| 8 | `WHISPER_MOB` | Шепот от NPC |
| 9 | `WHISPER_INFORM` | Уведомление об отправленном шепоте |
| 10 | `EMOTE` | /emote — пользовательская эмоция |
| 11 | `TEXT_EMOTE` | Стандартная эмоция (/dance, /wave и т.д.) |
| 12 | `MONSTER_SAY` | NPC — обычная фраза |
| 13 | `MONSTER_PARTY` | NPC — в группу |
| 14 | `MONSTER_YELL` | NPC — крик (боссы, квестовые NPC) |
| 15 | `MONSTER_WHISPER` | NPC — шепот игроку |
| 16 | `MONSTER_EMOTE` | NPC — эмоция |
| 17 | `CHANNEL` | Пользовательский/системный канал (/1, /2, Trade, LFG...) |
| 18 | `CHANNEL_JOIN` | Уведомление о входе в канал |
| 19 | `CHANNEL_LEAVE` | Уведомление о выходе из канала |
| 20 | `CHANNEL_LIST` | Список участников канала |
| 21 | `CHANNEL_NOTICE` | Уведомление канала |
| 22 | `CHANNEL_NOTICE_USER` | Уведомление канала о пользователе |
| 23 | `AFK` | /afk — сообщение "отошел" |
| 24 | `DND` | /dnd — сообщение "не беспокоить" |
| 25 | `IGNORED` | Сообщение от игнорируемого игрока |
| 26 | `SKILL` | Уведомление о навыке |
| 27 | `LOOT` | Уведомление о добыче |
| 28 | `SYSTEM` | Системное сообщение |

### 3.1 Какие типы обычно интересны для переводчика

Для перевода чата стоит фильтровать:

- **Чат игроков:** `SAY (1)`, `YELL (6)`, `PARTY (2)`, `RAID (3)`, `GUILD (4)`, `OFFICER (5)`, `WHISPER (7)`, `CHANNEL (17)`
- **Можно игнорировать:** `ADDON (0)`, `WHISPER_INFORM (9)`, `CHANNEL_JOIN/LEAVE/LIST/NOTICE (18–22)`, `SKILL (26)`, `LOOT (27)`, `SYSTEM (28)`, `TEXT_EMOTE (11)`

---

## 4. Вспомогательные оффсеты

### 4.1 Информация о локальном игроке

| Имя | Адрес | Тип | Описание |
|---|---|---|---|
| `PlayerName` | `0x00C79D18` | char[] | Имя текущего персонажа (null-terminated) |
| `LocalPlayerGuid` | `0x00CA1238` | uint64 | GUID текущего персонажа |

### 4.2 Name Store (Кэш имен по GUID)

WoW кэширует соответствие GUID → имя в хэш-таблице. Это полезно, если SenderName
не читается напрямую из структуры сообщения.

| Имя | Адрес/Смещение | Тип | Описание |
|---|---|---|---|
| `NameStoreBase` | `0x00C5D940` | ptr | Базовый адрес хэш-таблицы имен |
| `NameBase` | `+0x1C` | offset | Смещение к первому элементу в записи |
| `NameString` | `+0x20` | offset | Смещение к строке имени в записи |
| `NameMask` | `+0x24` | offset | Маска хэш-таблицы (для вычисления bucket) |

### 4.3 Object Manager (для продвинутого использования)

| Имя | Адрес | Тип | Описание |
|---|---|---|---|
| `CurMgrPointer` | `0x00C79CE0` | ptr | Указатель на текущий Object Manager |
| `CurMgrOffset` | `+0x2ED0` | offset | Смещение до Object Manager внутри структуры |

---

## 5. Важные замечания

### 5.1 Кодировка строк
- Строки в WoW 3.3.5a используют **UTF-8** кодировку
- Кириллица, корейский, китайский — все работает через UTF-8
- При чтении из памяти — читаем байты до `\0`, декодируем как UTF-8

### 5.2 Потокобезопасность
- WoW пишет в буфер из основного потока игры
- Внешний reader читает из своего процесса через `ReadProcessMemory`
- Теоретически возможна гонка (читаем пока пишется), но на практике
  записи атомарны для наших целей — сначала пишутся данные, потом обновляется счетчик

### 5.3 Разница между FormattedMessage и PlainText
- **FormattedMessage** (`0x003C`) — содержит цветовые коды, ссылки на предметы,
  имя отправителя в квадратных скобках и прочее форматирование WoW
  - Пример: `|Hplayer:Артас|h[Артас]|h says: Привет`
- **PlainText** (`0x0BF4`) — чистый текст без форматирования
  - Пример: `Привет`

Для перевода стоит использовать **PlainText**, а для отображения — комбинацию
имени отправителя + перевод PlainText.

---

## 6. Источники

- [AmeisenBot-3.3.5a — Offsets.cs](https://github.com/TheRaven-dev/AmeisenBot-3.3.5a) (GitHub)
- [WoW-3.3.5a-Bot forks — Offsets.cs](https://github.com/Zz9uk3/WoW-3.3.5a-Bot) (GitHub)
- [OwnedCore — C# Chat Buffer Memory (Updated)](https://www.ownedcore.com/forums/world-of-warcraft/world-of-warcraft-bots-programs/wow-memory-editing/280764-c-copypasta-chat-buffer-memory-updated.html)
- [OwnedCore — 3.3.5a 12340 Offsets](https://www.ownedcore.com/forums/world-of-warcraft/world-of-warcraft-bots-programs/wow-memory-editing/298984-3-3-5a-12340-offsets.html)
- [OwnedCore — 3.3.5 Offsets (DrakeFish dump)](https://www.ownedcore.com/forums/world-of-warcraft/world-of-warcraft-bots-programs/wow-memory-editing/298310-3-3-5-offsets.html)
- [OwnedCore — 3.3.5.12340 Info Dump Thread](https://www.ownedcore.com/forums/world-of-warcraft/world-of-warcraft-bots-programs/wow-memory-editing/300463-wow-3-3-5-12340-info-dump-thread.html)
- [Wowpedia — Chat](https://wowpedia.fandom.com/wiki/Chat)
- [WoWWiki — SendChatMessage API](https://wowwiki-archive.fandom.com/wiki/API_SendChatMessage)
