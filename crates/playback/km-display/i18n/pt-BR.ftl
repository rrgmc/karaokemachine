# O que a televisão diz.
#
# Tradução de `en.ftl`, que é onde toda mensagem é escrita primeiro.
#
# **Todo caractere aqui precisa existir na fonte embutida**, que só cobre o latino. O teste é
# `every_message_is_drawable`.
#
# **Estas palavras são lidas do sofá.** Curto vence completo: uma frase que precisa encolher dois
# degraus para caber é uma frase que ninguém lê nos quatro segundos em que fica na tela.

## O teclado
#
# Os dígitos não estão aqui de propósito — o português conta com os mesmos algarismos.
#
# `CLR` e `OK` ficam como estão: são as palavras impressas em todo teclado numérico de aparelho de
# karaokê, e traduzir `OK` para `ENTRA` seria menos legível, não mais.

keypad-clear = CLR
keypad-submit = OK

## A barra de controles

# Estas palavras são impressas em botões estreitos: a tira tem `STRIP_ASPECT` de largura por altura,
# e `MELODIA` já é a mais comprida que cabe. Curto vence exato.
transport-pause = PAUSA
transport-back = -10s
transport-forward = +10s
transport-next = PULAR
transport-again = REPETIR
transport-queue = FILA
transport-key-down = TOM -
transport-key-up = TOM +
transport-melody = MELODIA

## A tela de espera

idle-prompt = Digite o número da música
catalog-empty = Nenhuma música instalada
catalog-summary = { $songs ->
    [one] { $songs_display } música
   *[other] { $songs_display } músicas
 } · { $packages ->
    [one] { $packages_display } pacote
   *[other] { $packages_display } pacotes
 }

## Um problema permanente

notice-faults = { $count ->
    [one] { $count } problema
   *[other] { $count } problemas
 }: { $areas }
fault-packages = pacotes
fault-sound = som

## O aviso de modo de desenvolvimento

developer-debugging = MODO DEPURAÇÃO ATIVADO
developer-console = CONSOLE DEV ATIVADO

## A fila

queue-heading = Fila
queue-waiting = { $count ->
    [one] { $count } música esperando
   *[other] { $count } músicas esperando
 }
queue-empty = Nada na fila — digite o número de uma música
queue-overflow = e mais { $count }

## Enquanto uma música toca

next-up = a seguir: { $title }
no-lyrics = (esta música não tem letra)

## Os distintivos sobre a música

badge-key = tom { $semitones }
badge-tempo = ritmo { $ratio }x
badge-melody = melodia
badge-lyrics-hidden = sem letra

## O medidor de quadros

frames-heading = QUADROS
frames-measuring = medindo...
frames-draw = desenho
frames-present = envio
frames-interval = intervalo
frames-starved = sem dados
frames-dropped = perdidos
frames-late = atrasados
frames-xruns = falhas

## A música sob o medidor de quadros

song-heading = MÚSICA
song-kind-midi = midi, { $tracks } trilhas
song-kind-video = vídeo
song-kind-cdg = mp3+g
song-kind-ultrastar = ultrastar
song-kind-lrc = lrc

song-position = posição
song-gain = ganho
song-levelled = nivelado
song-levelled-package = pac { $lufs } LUFS
song-levelled-events = midi { $db } dB
song-levelled-off = desligado
song-levelled-none = nenhum

song-fixes = correções
song-fixes-none = nenhuma
song-fix-bank = banco { $count }
song-fix-mute = mudo { $count }

song-lyrics = letra
song-flavor-soft-karaoke = soft-karaoke
song-flavor-lyric-events = eventos de letra
song-flavor-named-text-track = trilha de texto
song-flavor-none = sem letra

song-damage = danos
song-damage-cut = cortadas { $count }
song-damage-gone = perdidas { $count }
song-damage-notes = notas { $count }

## O painel de conexão

connect-heading = Controle remoto
connect-local-only = O controle remoto só funciona nesta máquina
connect-local-only-detail = Escutando em { $address }. Defina o endereço para 0.0.0.0 para um celular conseguir conectar.
connect-no-network = Sem rede
connect-no-network-detail = Conecte esta máquina ao Wi-Fi ou à rede para usar um controle.
connect-unavailable = Controle remoto indisponível
connect-no-address = Nenhum endereço utilizável foi encontrado.
connect-factory-pin = · PIN { $pin }
connect-other-addresses = { $count ->
    [one] mais { $count } endereço
   *[other] mais { $count } endereços
 }
connect-browser-key = F11 abre o controle remoto no navegador
connect-browser-key-ctrl = Ctrl+F11 abre o controle remoto no navegador

## O número que alguém está digitando

number-invalid = número inválido
