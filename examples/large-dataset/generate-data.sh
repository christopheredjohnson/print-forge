#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
output=${1:-"$script_directory/data.csv"}
rows=${2:-10000}

case $rows in
    ''|*[!0-9]*|0)
        echo "row count must be a positive integer" >&2
        exit 1
        ;;
esac

printf '%s\n' 'record_id,first_name,last_name,company,street,city,state,postal_code,service,tracking_code,campaign_url' > "$output"

index=1
while [ "$index" -le "$rows" ]; do
    name_variant=$((index % 12))
    case $name_variant in
        0) first_name='Ada'; last_name='Lovelace' ;;
        1) first_name='Grace'; last_name='Hopper' ;;
        2) first_name='Katherine'; last_name='Johnson' ;;
        3) first_name='Margaret'; last_name='Hamilton' ;;
        4) first_name='Alan'; last_name='Turing' ;;
        5) first_name='Donald'; last_name='Knuth' ;;
        6) first_name='Barbara'; last_name='Liskov' ;;
        7) first_name='Edsger'; last_name='Dijkstra' ;;
        8) first_name='Radia'; last_name='Perlman' ;;
        9) first_name='James'; last_name='Gosling' ;;
        10) first_name='Ken'; last_name='Thompson' ;;
        11) first_name='Dennis'; last_name='Ritchie' ;;
    esac
    variant=$((index % 6))
    case $variant in
        0) city='New York'; state='NY'; service='GROUND' ;;
        1) city='Austin'; state='TX'; service='EXPRESS' ;;
        2) city='Seattle'; state='WA'; service='TWO-DAY' ;;
        3) city='Chicago'; state='IL'; service='GROUND' ;;
        4) city='Miami'; state='FL'; service='EXPRESS' ;;
        5) city='Los Angeles'; state='CA'; service='TWO-DAY' ;;
    esac
    record_id=$(printf 'PF%06d' "$index")
    tracking_code=$(printf 'PF26%012d' "$index")
    street_number=$((100 + index % 9800))
    postal_code=$(printf '%05d' $((10000 + index % 89999)))
    company_number=$(printf '%04d' $((index % 1000)))
    printf '%s\n' "$record_id,$first_name,$last_name,Forge Customer $company_number,$street_number Example Avenue,$city,$state,$postal_code,$service,$tracking_code,https://example.com/orders/$record_id" >> "$output"
    index=$((index + 1))
done

echo "wrote $rows rows to $output"
